use super::super::navigation_entry::{history_entries, navigation_entry_key_value};
use super::super::navigation_events::{
    NavigationDispatchOutcome, dispatch_navigation_success,
    run_navigation_precommit_deferred_handlers,
};
use super::super::navigation_lifecycle::finish_navigation_error_events;
use super::super::navigation_result::{
    navigation_dom_exception, perform_navigation_scroll_if_needed, suppress_unhandled_rejection,
};
use super::super::navigation_window::{
    navigation_document_has_opaque_origin, navigation_document_is_active, runtime_top_window_owner,
    runtime_window_owner, window_history_for_holder, window_location_for_holder,
    window_navigation_for_holder, window_task_target_for_runtime_owner,
};
use super::super::*;
use super::apply::dispatch_history_entry_post_commit_events;
use super::results::reject_pending_navigation_results;
use crate::native_bridge::PendingHistoryTraversal;
use crate::util::{get_private_value, set_private_value};
use moli_webapi_declare::WebApiObject;

const TRAVERSAL_INTERCEPT_ACTIVE_SLOT: &str = "__lmTraversalInterceptActive";
const TRAVERSAL_INTERCEPT_NAVIGATION_SLOT: &str = "__lmTraversalInterceptNavigation";
const TRAVERSAL_INTERCEPT_SIGNAL_SLOT: &str = "__lmTraversalInterceptSignal";
const TRAVERSAL_INTERCEPT_FINISHED_RESOLVERS_SLOT: &str = "__lmTraversalInterceptFinishedResolvers";
const TRAVERSAL_INTERCEPT_VALUE_SLOT: &str = "__lmTraversalInterceptValue";
const TRAVERSAL_INTERCEPT_URL_SLOT: &str = "__lmTraversalInterceptUrl";
const TRAVERSAL_INTERCEPT_PROMISE_SLOT: &str = "__lmTraversalInterceptPromise";
const NAVIGATION_ACTIVE_TRAVERSAL_INTERCEPT_SLOT: &str = "__lmNavigationActiveTraversalIntercept";

#[derive(WebApiObject)]
#[webapi(plain)]
struct TraversalInterceptSettlementDataDeclaration<'scope> {
    #[webapi(slot = TRAVERSAL_INTERCEPT_ACTIVE_SLOT)]
    active: bool,

    #[webapi(slot = TRAVERSAL_INTERCEPT_NAVIGATION_SLOT)]
    navigation: v8::Local<'scope, v8::Object>,

    #[webapi(slot = TRAVERSAL_INTERCEPT_SIGNAL_SLOT)]
    signal: Option<v8::Local<'scope, v8::Object>>,

    #[webapi(slot = TRAVERSAL_INTERCEPT_FINISHED_RESOLVERS_SLOT)]
    finished_resolvers: v8::Local<'scope, v8::Array>,

    #[webapi(slot = TRAVERSAL_INTERCEPT_VALUE_SLOT)]
    value: v8::Local<'scope, v8::Value>,

    #[webapi(slot = TRAVERSAL_INTERCEPT_URL_SLOT)]
    url: v8::Local<'scope, v8::String>,
}

fn navigation_active_traversal_intercept<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(
        scope,
        navigation,
        NAVIGATION_ACTIVE_TRAVERSAL_INTERCEPT_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn set_navigation_active_traversal_intercept<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    data: v8::Local<'s, v8::Object>,
) {
    set_private_value(
        scope,
        navigation,
        NAVIGATION_ACTIVE_TRAVERSAL_INTERCEPT_SLOT,
        data.into(),
    );
}

fn clear_navigation_active_traversal_intercept_if_current<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    data: v8::Local<'s, v8::Object>,
) {
    if navigation_active_traversal_intercept(scope, navigation)
        .is_some_and(|active| active.strict_equals(data.into()))
    {
        set_private_value(
            scope,
            navigation,
            NAVIGATION_ACTIVE_TRAVERSAL_INTERCEPT_SLOT,
            v8::undefined(scope).into(),
        );
    }
}

pub(in crate::context_bootstrap) fn pending_history_traversal_target_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
) -> Option<u32> {
    let owner = runtime_window_owner(scope, history);
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let host = unsafe { &*host_ptr };
    let target = window_task_target_for_runtime_owner(scope, host, owner)?;
    host.pending_history_traversal_target_index(target)
}

pub(in crate::context_bootstrap) fn route_history_traversal_task(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    producer: crate::page_task_queue::RendererPageHistoryTraversalProducer,
) {
    let task_id = producer.task_id();
    if producer.send().is_ok() {
        return;
    }
    let Some(queued) = host.take_pending_history_traversal_task(task_id) else {
        return;
    };
    let results = match &queued.action {
        crate::native_bridge::PendingHistoryTraversalAction::SameDocument(traversal) => {
            traversal.results.as_slice()
        }
        crate::native_bridge::PendingHistoryTraversalAction::ChildCrossDocument(traversal) => {
            traversal.results.as_slice()
        }
    };
    if results.is_empty() {
        return;
    }
    let error = navigation_dom_exception(scope, "Navigation was canceled", "AbortError");
    reject_pending_navigation_results(scope, results, error);
}

pub(in crate::context_bootstrap) fn apply_pending_history_traversal(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    traversal: PendingHistoryTraversal,
) {
    let plan = history_traversal_target_window(scope, host, traversal.target).and_then(|owner| {
        let history = window_history_for_holder(scope, owner)?;
        let entry = history_entries(scope, history)?
            .get_index(scope, traversal.target_index)?
            .try_into()
            .ok()?;
        if traversal
            .target_key
            .as_ref()
            .is_some_and(|key| navigation_entry_key_value(scope, entry).as_ref() != Some(key))
        {
            return None;
        }
        let step = traversal
            .joint_step
            .or_else(|| super::super::session_history::step_for_entry(scope, owner, entry))?;
        super::super::navigation_traversal_plan::JointTraversalPlan::resolve(scope, owner, step)
    });
    let Some(plan) = plan else {
        let error = navigation_dom_exception(scope, "Navigation was canceled", "AbortError");
        reject_pending_navigation_results(scope, &traversal.results, error);
        return;
    };
    let info = traversal
        .info
        .as_ref()
        .map(|value| v8::Local::new(scope, value));
    super::super::navigation_traversal_coordinator::execute(scope, plan, info, traversal.results);
}

pub(in crate::context_bootstrap) fn prepare_history_participant<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    applied: &super::apply::AppliedHistoryEntry<'s>,
    outcome: &NavigationDispatchOutcome<'s>,
    finished_resolvers: v8::Local<'s, v8::Array>,
) -> Option<v8::Local<'s, v8::Object>> {
    if !outcome.intercepted {
        return None;
    }
    let navigation = window_navigation_for_holder(scope, applied.owner)?;
    Some(set_active_traversal_intercept_settlement(
        scope,
        navigation,
        outcome.signal,
        finished_resolvers,
        applied.resolved_entry,
        &applied.url,
    ))
}

pub(in crate::context_bootstrap) fn finish_history_participant<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    applied: &super::apply::AppliedHistoryEntry<'s>,
    outcome: NavigationDispatchOutcome<'s>,
    finished_resolvers: v8::Local<'s, v8::Array>,
    settlement: Option<v8::Local<'s, v8::Object>>,
) {
    if settlement.is_some_and(|data| !traversal_intercept_is_active(scope, data.into())) {
        return;
    }
    if !outcome.intercepted {
        crate::script_vm::perform_microtask_checkpoint_and_report_pending_promise_rejections(scope);
        if !navigation_document_has_opaque_origin(scope, applied.owner)
            && let Some(navigation) = window_navigation_for_holder(scope, applied.owner)
        {
            let active_scroll = super::super::navigation_events::navigation_has_active_scroll_event(
                scope, navigation,
            );
            if active_scroll
                || !super::super::navigation_entry::restore_current_navigation_entry_scroll_position(
                    scope,
                    applied.owner,
                )
            {
                perform_navigation_scroll_if_needed(scope, navigation, &applied.url, true);
            }
            dispatch_navigation_success(scope, navigation);
        }
        resolve_resolver_array(scope, finished_resolvers, applied.resolved_entry);
        crate::script_vm::perform_microtask_checkpoint_and_report_pending_promise_rejections(scope);
        dispatch_history_entry_post_commit_events(scope, applied, true);
        return;
    }
    let Some(navigation) = window_navigation_for_holder(scope, applied.owner) else {
        dispatch_history_entry_post_commit_events(scope, applied, true);
        resolve_resolver_array(scope, finished_resolvers, applied.resolved_entry);
        return;
    };
    let (error, result) = if outcome.intercepted {
        if let Some(event) = outcome.precommit_event {
            run_navigation_precommit_deferred_handlers(scope, event)
        } else {
            (outcome.intercept_error, outcome.intercept_result)
        }
    } else {
        (None, None)
    };
    suppress_intercept_result_unhandled_rejection(scope, result);
    dispatch_history_entry_post_commit_events(scope, applied, true);
    if let Some(data) = settlement {
        if !traversal_intercept_is_active(scope, data.into()) {
            return;
        }
        set_traversal_intercept_inactive(scope, navigation, data.into());
    }
    if let Some(error) = error {
        finish_navigation_error_events(scope, navigation, error, &applied.url);
        reject_resolver_array(scope, finished_resolvers, error, true);
        if let Some(signal) = outcome.signal {
            crate::native_bridge::abort::abort_signal(scope, signal, error);
        }
        return;
    }
    if let Some(result) = result
        && queue_pending_traversal_intercept_settlement(
            scope,
            navigation,
            outcome.signal,
            finished_resolvers,
            applied.resolved_entry,
            &applied.url,
            result,
        )
    {
        return;
    }
    perform_navigation_scroll_if_needed(scope, navigation, &applied.url, true);
    dispatch_navigation_success(scope, navigation);
    resolve_resolver_array(scope, finished_resolvers, applied.resolved_entry);
}

fn history_traversal_target_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &mut JsContextHost,
    target: crate::native_bridge::WindowTaskTarget,
) -> Option<v8::Local<'s, v8::Object>> {
    match target.dispatch_scope() {
        crate::native_bridge::OwnerDispatchScope::Top => {
            Some(scope.get_current_context().global(scope))
        }
        crate::native_bridge::OwnerDispatchScope::Child(child_handle) => {
            host.child_browsing_context_window_wrapper(scope, child_handle)
        }
        crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id) => {
            host.lightweight_popup_window(scope, popup_id)
        }
    }
}

pub(in crate::context_bootstrap) fn finished_resolver_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    results: &[crate::native_bridge::PendingNavigationResult],
) -> v8::Local<'s, v8::Array> {
    let finished = v8::Array::new(scope, results.len() as i32);
    for (index, result) in results.iter().enumerate() {
        let finished_resolver = v8::Local::new(scope, &result.finished_resolver);
        let _ = finished.set_index(scope, index as u32, finished_resolver.into());
    }
    finished
}

fn suppress_intercept_result_unhandled_rejection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: Option<v8::Local<'s, v8::Value>>,
) {
    if let Some(result) = result
        && let Ok(promise) = v8::Local::<v8::Promise>::try_from(result)
    {
        suppress_unhandled_rejection(scope, promise);
    }
}

fn queue_pending_traversal_intercept_settlement<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    signal: Option<v8::Local<'s, v8::Object>>,
    finished_resolvers: v8::Local<'s, v8::Array>,
    resolved_value: v8::Local<'s, v8::Value>,
    url: &str,
    result: v8::Local<'s, v8::Value>,
) -> bool {
    let Some(result_object) = v8::Local::<v8::Object>::try_from(result).ok() else {
        return false;
    };
    let Some(then) = result_object
        .get(scope, v8str(scope, "then").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return false;
    };
    let data = set_active_traversal_intercept_settlement(
        scope,
        navigation,
        signal,
        finished_resolvers,
        resolved_value,
        url,
    );
    if let Ok(promise) = v8::Local::<v8::Promise>::try_from(result) {
        suppress_unhandled_rejection(scope, promise);
        set_private_value(scope, data, TRAVERSAL_INTERCEPT_PROMISE_SLOT, result);
    }
    let Some(on_fulfilled) = v8::Function::builder(traversal_intercept_fulfilled_callback)
        .data(data.into())
        .build(scope)
    else {
        return false;
    };
    let Some(on_rejected) = v8::Function::builder(traversal_intercept_rejected_callback)
        .data(data.into())
        .build(scope)
    else {
        return false;
    };
    then.call(scope, result, &[on_fulfilled.into(), on_rejected.into()])
        .is_some()
}

fn set_active_traversal_intercept_settlement<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    signal: Option<v8::Local<'s, v8::Object>>,
    finished_resolvers: v8::Local<'s, v8::Array>,
    resolved_value: v8::Local<'s, v8::Value>,
    url: &str,
) -> v8::Local<'s, v8::Object> {
    let url = v8_string(scope, url).unwrap_or_else(|| v8::String::empty(scope));
    let data = TraversalInterceptSettlementDataDeclaration {
        active: true,
        navigation,
        signal,
        finished_resolvers,
        value: resolved_value,
        url,
    }
    .bind(scope)
    .expect("traversal intercept settlement data should bind");
    set_navigation_active_traversal_intercept(scope, navigation, data);
    data
}

fn traversal_intercept_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<(
    v8::Local<'s, v8::Object>,
    Option<v8::Local<'s, v8::Object>>,
    v8::Local<'s, v8::Array>,
    v8::Local<'s, v8::Value>,
    String,
    Option<v8::Local<'s, v8::Promise>>,
)> {
    let data = v8::Local::<v8::Object>::try_from(data).ok()?;
    if !traversal_private_bool(scope, data, TRAVERSAL_INTERCEPT_ACTIVE_SLOT) {
        return None;
    }
    let navigation = get_private_value(scope, data, TRAVERSAL_INTERCEPT_NAVIGATION_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let signal = get_private_value(scope, data, TRAVERSAL_INTERCEPT_SIGNAL_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let finished_resolvers =
        get_private_value(scope, data, TRAVERSAL_INTERCEPT_FINISHED_RESOLVERS_SLOT)
            .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())?;
    let resolved_value = get_private_value(scope, data, TRAVERSAL_INTERCEPT_VALUE_SLOT)
        .unwrap_or_else(|| v8::undefined(scope).into());
    let url = get_private_value(scope, data, TRAVERSAL_INTERCEPT_URL_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    let promise = get_private_value(scope, data, TRAVERSAL_INTERCEPT_PROMISE_SLOT)
        .and_then(|value| v8::Local::<v8::Promise>::try_from(value).ok());
    Some((
        navigation,
        signal,
        finished_resolvers,
        resolved_value,
        url,
        promise,
    ))
}

fn traversal_intercept_is_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> bool {
    v8::Local::<v8::Object>::try_from(data)
        .ok()
        .is_some_and(|data| traversal_private_bool(scope, data, TRAVERSAL_INTERCEPT_ACTIVE_SLOT))
}

fn traversal_private_bool<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> bool {
    get_private_value(scope, data, slot).is_some_and(|value| value.boolean_value(scope))
}

fn set_traversal_intercept_inactive<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    data: v8::Local<'s, v8::Value>,
) {
    if let Ok(data) = v8::Local::<v8::Object>::try_from(data) {
        set_private_value(
            scope,
            data,
            TRAVERSAL_INTERCEPT_ACTIVE_SLOT,
            v8::Boolean::new(scope, false).into(),
        );
        clear_navigation_active_traversal_intercept_if_current(scope, navigation, data);
    }
}

pub(in crate::context_bootstrap) fn cancel_active_history_traversal_intercept_settlement<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(data) = navigation_active_traversal_intercept(scope, navigation)
        .filter(|data| traversal_intercept_is_active(scope, (*data).into()))
    else {
        return false;
    };
    let Some((navigation, signal, finished_resolvers, _, url, _)) =
        traversal_intercept_data(scope, data.into())
    else {
        return false;
    };
    set_traversal_intercept_inactive(scope, navigation, data.into());
    let error = navigation_dom_exception(scope, "Navigation was canceled", "AbortError");
    if let Some(signal) = signal {
        crate::native_bridge::abort::abort_signal(scope, signal, error);
    }
    finish_navigation_error_events(scope, navigation, error, &url);
    reject_resolver_array(scope, finished_resolvers, error, true);
    true
}

fn traversal_intercept_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((navigation, signal, finished_resolvers, resolved_value, url, _)) =
        traversal_intercept_data(scope, args.data())
    else {
        return;
    };
    set_traversal_intercept_inactive(scope, navigation, args.data());
    let owner = runtime_window_owner(scope, navigation);
    if !navigation_document_is_active(scope, owner) {
        let error = navigation_dom_exception(scope, "Navigation was canceled", "AbortError");
        if let Some(signal) = signal {
            crate::native_bridge::abort::abort_signal(scope, signal, error);
        }
        let top_owner = runtime_top_window_owner(scope, owner);
        let filename = window_location_for_holder(scope, top_owner)
            .and_then(|location| {
                super::super::location_runtime::location_href_slot(scope, location)
            })
            .unwrap_or(url);
        finish_navigation_error_events(scope, navigation, error, &filename);
        reject_resolver_array(scope, finished_resolvers, error, true);
        return;
    }
    perform_navigation_scroll_if_needed(scope, navigation, &url, true);
    dispatch_navigation_success(scope, navigation);
    resolve_resolver_array(scope, finished_resolvers, resolved_value);
}

fn traversal_intercept_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((navigation, signal, finished_resolvers, _, url, promise)) =
        traversal_intercept_data(scope, args.data())
    else {
        return;
    };
    set_traversal_intercept_inactive(scope, navigation, args.data());
    let error = promise
        .filter(|promise| promise.state() == v8::PromiseState::Rejected)
        .map(|promise| promise.result(scope))
        .unwrap_or_else(|| args.get(0));
    if let Some(signal) = signal {
        crate::native_bridge::abort::abort_signal(scope, signal, error);
    }
    finish_navigation_error_events(scope, navigation, error, &url);
    reject_resolver_array(scope, finished_resolvers, error, true);
}

pub(in crate::context_bootstrap) fn resolve_resolver_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolvers: v8::Local<'s, v8::Array>,
    value: v8::Local<'s, v8::Value>,
) {
    for index in 0..resolvers.length() {
        let Some(resolver) = resolvers
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .map(|object| unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(object) })
        else {
            continue;
        };
        let _ = resolver.resolve(scope, value);
    }
}

pub(in crate::context_bootstrap) fn reject_resolver_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolvers: v8::Local<'s, v8::Array>,
    error: v8::Local<'s, v8::Value>,
    suppress_unhandled: bool,
) {
    for index in 0..resolvers.length() {
        let Some(resolver) = resolvers
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .map(|object| unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(object) })
        else {
            continue;
        };
        let _ = resolver.reject(scope, error);
        if suppress_unhandled {
            suppress_unhandled_rejection(scope, resolver.get_promise(scope));
        }
    }
}
