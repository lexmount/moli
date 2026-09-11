use super::navigation_activation::{
    clear_navigation_transition, resolve_navigation_transition_committed,
    schedule_settle_navigation_transition,
};
use super::navigation_events::{
    NAVIGATE_EVENT_SCROLL_AFTER_TRANSITION_SLOT, NAVIGATE_EVENT_SCROLL_CALLED_SLOT,
    clear_navigation_focus_reset_epoch, clear_navigation_scroll_state, dispatch_navigation_error,
    dispatch_navigation_success, navigation_active_scroll_event, navigation_focus_reset_epoch,
    navigation_scroll_target_href,
};
use super::*;
use crate::native_bridge::element::{
    process_post_parse_autofocus, scroll_to_url_fragment_or_top, update_focus,
};
use crate::{native_bridge::NavigationAttemptId, util::get_private_value};
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(record)]
struct NavigationFinishedResolveDataDeclaration<'scope> {
    #[webapi(slot = NAVIGATION_FINISH_RESOLVE_SLOT)]
    resolve: v8::Local<'scope, v8::Function>,

    #[webapi(slot = NAVIGATION_FINISH_VALUE_SLOT)]
    value: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(record)]
struct NavigationFinishedRejectDataDeclaration<'scope> {
    #[webapi(slot = NAVIGATION_FINISH_REJECT_SLOT)]
    reject: v8::Local<'scope, v8::Function>,

    #[webapi(slot = NAVIGATION_FINISH_VALUE_SLOT)]
    value: v8::Local<'scope, v8::Value>,
}

pub(super) fn enqueue_navigation_lifecycle_microtask<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: v8::Local<'s, v8::Function>,
) {
    scope.enqueue_microtask(callback);
}

pub(super) fn begin_navigation_attempt<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: &'static str,
) -> Option<NavigationAttemptId> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    Some(unsafe { &mut *host_ptr }.begin_navigation_lifecycle_attempt(kind))
}

pub(super) fn navigation_attempt_is_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    attempt_id: NavigationAttemptId,
) -> bool {
    context_host_ptr_from_global_bridge(scope).is_some_and(|host_ptr| {
        unsafe { &*host_ptr }.navigation_lifecycle_attempt_is_active(attempt_id)
    })
}

pub(super) fn complete_navigation_attempt<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    attempt_id: NavigationAttemptId,
) {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        unsafe { &mut *host_ptr }.complete_navigation_lifecycle_attempt(attempt_id);
    }
}

pub(super) fn cancel_navigation_attempt<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    attempt_id: NavigationAttemptId,
) {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        unsafe { &mut *host_ptr }.cancel_navigation_lifecycle_attempt(attempt_id);
    }
}

pub(super) fn navigation_attempt_id_from_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<NavigationAttemptId> {
    let value = get_private_value(scope, object, slot)
        .filter(|value| !value.is_undefined())
        .or_else(|| object.get(scope, v8str(scope, slot).into()))?;
    let value = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (raw, lossless) = value.u64_value();
    lossless
        .then_some(raw)
        .and_then(NavigationAttemptId::from_raw)
}

pub(super) fn finish_navigation_success_events<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    href: &str,
) {
    perform_navigation_scroll_if_needed(scope, navigation, href, true);
    reset_navigation_focus_if_unchanged(scope, navigation);
    dispatch_navigation_success(scope, navigation);
}

pub(super) fn finish_navigation_error_events<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
    href: &str,
) {
    perform_navigation_scroll_if_needed(scope, navigation, href, false);
    reset_navigation_focus_if_unchanged(scope, navigation);
    dispatch_navigation_error(scope, navigation, error, href);
}

pub(super) fn settle_navigation_committed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    resolve: v8::Local<'s, v8::Function>,
    value: v8::Local<'s, v8::Value>,
) {
    let receiver = v8::undefined(scope).into();
    let _ = resolve.call(scope, receiver, &[value]);
    resolve_navigation_transition_committed(scope, navigation, value);
}

pub(super) fn settle_navigation_finished_resolved_immediately<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolve: v8::Local<'s, v8::Function>,
    value: v8::Local<'s, v8::Value>,
) {
    let receiver = v8::undefined(scope).into();
    let _ = resolve.call(scope, receiver, &[value]);
}

pub(super) fn settle_navigation_finished_resolved_after_reactions<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolve: v8::Local<'s, v8::Function>,
    value: v8::Local<'s, v8::Value>,
) {
    schedule_navigation_finished_resolve(scope, resolve, value);
}

pub(super) fn settle_navigation_finished_rejected<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reject: v8::Local<'s, v8::Function>,
    error: v8::Local<'s, v8::Value>,
) {
    let receiver = v8::undefined(scope).into();
    let _ = reject.call(scope, receiver, &[error]);
}

pub(super) fn settle_navigation_finished_rejected_after_reactions<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reject: v8::Local<'s, v8::Function>,
    error: v8::Local<'s, v8::Value>,
) {
    let data = NavigationFinishedRejectDataDeclaration::new(reject, error)
        .bind(scope)
        .expect("navigation finished reject data should bind");
    let Some(callback) = v8::Function::builder(navigation_finished_reject_callback)
        .data(data.into())
        .build(scope)
    else {
        settle_navigation_finished_rejected(scope, reject, error);
        return;
    };
    enqueue_navigation_lifecycle_microtask(scope, callback);
}

pub(super) fn settle_navigation_transition_finished<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    transition_resolver: Option<v8::Global<v8::PromiseResolver>>,
    error: Option<v8::Local<'s, v8::Value>>,
) {
    if transition_resolver.is_some() {
        clear_navigation_transition(scope, navigation);
    }
    if let Some(transition_resolver) = transition_resolver {
        let transition_resolver = v8::Local::new(scope, transition_resolver);
        settle_navigation_transition_finished_local(
            scope,
            navigation,
            Some(transition_resolver),
            error,
        );
    }
}

pub(super) fn settle_navigation_transition_finished_local<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    transition_resolver: Option<v8::Local<'s, v8::PromiseResolver>>,
    error: Option<v8::Local<'s, v8::Value>>,
) {
    if transition_resolver.is_some() {
        clear_navigation_transition(scope, navigation);
    }
    if let Some(transition_resolver) = transition_resolver {
        schedule_settle_navigation_transition(scope, navigation, transition_resolver, error);
    }
}

pub(super) fn perform_navigation_scroll_if_needed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    fallback_href: &str,
    succeeded: bool,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let active_event = navigation_active_scroll_event(scope, navigation);
    let target_href = navigation_scroll_target_href(scope, navigation);
    clear_navigation_scroll_state(scope, navigation);
    if !succeeded {
        return;
    }
    if let Some(event) = active_event {
        if get_private_value(scope, event, NAVIGATE_EVENT_SCROLL_CALLED_SLOT)
            .is_some_and(|value| value.is_true())
        {
            return;
        }
        if get_private_value(scope, event, NAVIGATE_EVENT_SCROLL_AFTER_TRANSITION_SLOT)
            .is_some_and(|value| !value.is_true())
        {
            return;
        }
        let Some(target_href) = target_href else {
            return;
        };
        let owner = super::navigation_window::runtime_window_owner(scope, navigation);
        if object_string_property(scope, event, "navigationType").as_deref() == Some("traverse")
            && super::navigation_entry::restore_current_navigation_entry_scroll_position(
                scope, owner,
            )
        {
            return;
        }
        if let Err(error) = scroll_to_url_fragment_or_top(scope, host_ptr, &target_href) {
            tracing::warn!(%error, "failed to resolve navigation fragment geometry");
        }
        return;
    }
    if !fallback_href.is_empty()
        && let Err(error) = scroll_to_url_fragment_or_top(scope, host_ptr, fallback_href)
    {
        tracing::warn!(%error, "failed to resolve navigation fragment geometry");
    }
}

pub(super) fn reset_navigation_focus_if_unchanged<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let Some(expected_epoch) = navigation_focus_reset_epoch(scope, navigation) else {
        return;
    };
    clear_navigation_focus_reset_epoch(scope, navigation);
    if unsafe { &*host_ptr }.focus_change_epoch() != expected_epoch {
        return;
    }
    update_focus(scope, host_ptr, None);
    let _ = process_post_parse_autofocus(scope, host_ptr);
}

const NAVIGATION_FINISH_RESOLVE_SLOT: &str = "__lmNavigationFinishResolve";
const NAVIGATION_FINISH_REJECT_SLOT: &str = "__lmNavigationFinishReject";
const NAVIGATION_FINISH_VALUE_SLOT: &str = "__lmNavigationFinishValue";

fn schedule_navigation_finished_resolve<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolve: v8::Local<'s, v8::Function>,
    value: v8::Local<'s, v8::Value>,
) {
    let data = NavigationFinishedResolveDataDeclaration::new(resolve, value)
        .bind(scope)
        .expect("navigation finished resolve data should bind");
    let Some(callback) = v8::Function::builder(navigation_finished_resolve_callback)
        .data(data.into())
        .build(scope)
    else {
        let receiver = v8::undefined(scope).into();
        let _ = resolve.call(scope, receiver, &[value]);
        return;
    };
    enqueue_navigation_lifecycle_microtask(scope, callback);
}

fn navigation_finished_resolve_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(data) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    let Some(resolve) = get_private_value(scope, data, NAVIGATION_FINISH_RESOLVE_SLOT)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return;
    };
    let value = get_private_value(scope, data, NAVIGATION_FINISH_VALUE_SLOT)
        .unwrap_or_else(|| v8::undefined(scope).into());
    let receiver = v8::undefined(scope).into();
    let _ = resolve.call(scope, receiver, &[value]);
}

fn navigation_finished_reject_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(data) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    let Some(reject) = get_private_value(scope, data, NAVIGATION_FINISH_REJECT_SLOT)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return;
    };
    let error = get_private_value(scope, data, NAVIGATION_FINISH_VALUE_SLOT)
        .unwrap_or_else(|| v8::undefined(scope).into());
    let receiver = v8::undefined(scope).into();
    let _ = reject.call(scope, receiver, &[error]);
}
