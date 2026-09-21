use super::super::navigation_activation::{
    install_navigation_transition, navigation_transition_matches_resolver,
    precommit_transition_resolver_from_event, reject_navigation_transition_committed,
    resolve_navigation_transition_committed, take_navigation_transition_committed_resolver,
};
use super::super::navigation_entry::{
    history_entries, history_index, navigation_current_entry, navigation_entry_key_value,
};
use super::super::navigation_events::{
    NavigationDispatchOutcome, dispatch_navigation_success,
    dispatch_navigation_traverse_event_with_outcome, finish_navigation_precommit,
    mark_navigation_outcome_default_prevented, run_navigation_precommit_deferred_handlers,
};
use super::super::navigation_lifecycle::{
    finish_navigation_error_events, settle_navigation_transition_finished_local,
};
use super::super::navigation_result::{
    navigation_dom_exception, perform_navigation_scroll_if_needed, suppress_unhandled_rejection,
};
use super::super::navigation_window::{
    navigation_document_has_opaque_origin, navigation_document_is_active, runtime_top_window_owner,
    runtime_window_owner, window_history_for_holder, window_location_for_holder,
    window_navigation_for_holder, window_task_target_for_runtime_owner,
};
use super::super::*;
use super::apply::{
    apply_history_entry, apply_history_entry_commit, dispatch_history_entry_currententrychange,
    dispatch_history_entry_post_commit_events,
};
use super::results::{reject_pending_navigation_results, resolve_pending_navigation_results};
use crate::native_bridge::PendingHistoryTraversal;
use crate::script_cleanup::ScriptExecutionScope;
use crate::util::{get_private_value, set_private_value};
use moli_webapi_declare::WebApiObject;

const TRAVERSAL_PRECOMMIT_ACTIVE_SLOT: &str = "__lmTraversalPrecommitActive";
const TRAVERSAL_PRECOMMIT_NAVIGATION_SLOT: &str = "__lmTraversalPrecommitNavigation";
const TRAVERSAL_PRECOMMIT_HISTORY_SLOT: &str = "__lmTraversalPrecommitHistory";
const TRAVERSAL_PRECOMMIT_EVENT_SLOT: &str = "__lmTraversalPrecommitEvent";
const TRAVERSAL_PRECOMMIT_SIGNAL_SLOT: &str = "__lmTraversalPrecommitSignal";
const TRAVERSAL_PRECOMMIT_TARGET_INDEX_SLOT: &str = "__lmTraversalPrecommitTargetIndex";
const TRAVERSAL_PRECOMMIT_PROMISE_SLOT: &str = "__lmTraversalPrecommitPromise";
const TRAVERSAL_PRECOMMIT_COMMITTED_RESOLVERS_SLOT: &str =
    "__lmTraversalPrecommitCommittedResolvers";
const TRAVERSAL_PRECOMMIT_FINISHED_RESOLVERS_SLOT: &str = "__lmTraversalPrecommitFinishedResolvers";
const NAVIGATION_PENDING_TRAVERSAL_PRECOMMIT_SLOT: &str = "__lmNavigationPendingTraversalPrecommit";
const TRAVERSAL_INTERCEPT_ACTIVE_SLOT: &str = "__lmTraversalInterceptActive";
const TRAVERSAL_INTERCEPT_NAVIGATION_SLOT: &str = "__lmTraversalInterceptNavigation";
const TRAVERSAL_INTERCEPT_SIGNAL_SLOT: &str = "__lmTraversalInterceptSignal";
const TRAVERSAL_INTERCEPT_FINISHED_RESOLVERS_SLOT: &str = "__lmTraversalInterceptFinishedResolvers";
const TRAVERSAL_INTERCEPT_VALUE_SLOT: &str = "__lmTraversalInterceptValue";
const TRAVERSAL_INTERCEPT_URL_SLOT: &str = "__lmTraversalInterceptUrl";
const TRAVERSAL_INTERCEPT_PROMISE_SLOT: &str = "__lmTraversalInterceptPromise";
const NAVIGATION_ACTIVE_TRAVERSAL_INTERCEPT_SLOT: &str = "__lmNavigationActiveTraversalIntercept";
const TRAVERSAL_TRANSITION_RESOLVER_SLOT: &str = "__lmTraversalTransitionResolver";

#[derive(WebApiObject)]
#[webapi(plain)]
struct TraversalPrecommitDataDeclaration<'scope> {
    #[webapi(slot = TRAVERSAL_PRECOMMIT_ACTIVE_SLOT)]
    active: bool,

    #[webapi(slot = TRAVERSAL_PRECOMMIT_NAVIGATION_SLOT)]
    navigation: v8::Local<'scope, v8::Object>,

    #[webapi(slot = TRAVERSAL_PRECOMMIT_HISTORY_SLOT)]
    history: v8::Local<'scope, v8::Object>,

    #[webapi(slot = TRAVERSAL_PRECOMMIT_EVENT_SLOT)]
    event: v8::Local<'scope, v8::Object>,

    #[webapi(slot = TRAVERSAL_PRECOMMIT_SIGNAL_SLOT)]
    signal: Option<v8::Local<'scope, v8::Object>>,

    #[webapi(slot = TRAVERSAL_PRECOMMIT_TARGET_INDEX_SLOT)]
    target_index: u32,

    #[webapi(slot = TRAVERSAL_PRECOMMIT_PROMISE_SLOT)]
    promise: Option<v8::Local<'scope, v8::Value>>,

    #[webapi(slot = TRAVERSAL_PRECOMMIT_COMMITTED_RESOLVERS_SLOT)]
    committed_resolvers: v8::Local<'scope, v8::Array>,

    #[webapi(slot = TRAVERSAL_PRECOMMIT_FINISHED_RESOLVERS_SLOT)]
    finished_resolvers: v8::Local<'scope, v8::Array>,

    #[webapi(slot = TRAVERSAL_TRANSITION_RESOLVER_SLOT)]
    transition_resolver: Option<v8::Local<'scope, v8::PromiseResolver>>,
}

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

    #[webapi(slot = TRAVERSAL_TRANSITION_RESOLVER_SLOT)]
    transition_resolver: Option<v8::Local<'scope, v8::PromiseResolver>>,
}

struct InterceptedHistoryTraversal<'s> {
    navigation: v8::Local<'s, v8::Object>,
    history: v8::Local<'s, v8::Object>,
    target_index: u32,
    event: Option<v8::Local<'s, v8::Object>>,
    signal: Option<v8::Local<'s, v8::Object>>,
    intercept_result: Option<v8::Local<'s, v8::Value>>,
    committed_resolvers: v8::Local<'s, v8::Array>,
    finished_resolvers: v8::Local<'s, v8::Array>,
    transition_resolver: Option<v8::Local<'s, v8::PromiseResolver>>,
}

fn traversal_transition_resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::PromiseResolver>> {
    let data = v8::Local::<v8::Object>::try_from(data).ok()?;
    get_private_value(scope, data, TRAVERSAL_TRANSITION_RESOLVER_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .map(|object| unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(object) })
}

fn navigation_pending_traversal_precommit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(
        scope,
        navigation,
        NAVIGATION_PENDING_TRAVERSAL_PRECOMMIT_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn set_navigation_pending_traversal_precommit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    data: v8::Local<'s, v8::Object>,
) {
    set_private_value(
        scope,
        navigation,
        NAVIGATION_PENDING_TRAVERSAL_PRECOMMIT_SLOT,
        data.into(),
    );
}

fn clear_navigation_pending_traversal_precommit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) {
    set_private_value(
        scope,
        navigation,
        NAVIGATION_PENDING_TRAVERSAL_PRECOMMIT_SLOT,
        v8::undefined(scope).into(),
    );
}

fn clear_navigation_pending_traversal_precommit_if_current<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    data: v8::Local<'s, v8::Object>,
) {
    if navigation_pending_traversal_precommit(scope, navigation)
        .is_some_and(|active| active.strict_equals(data.into()))
    {
        clear_navigation_pending_traversal_precommit(scope, navigation);
    }
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
        crate::native_bridge::PendingHistoryTraversalAction::ByDelta { .. } => &[],
        crate::native_bridge::PendingHistoryTraversalAction::SameDocument(traversal) => {
            traversal.results.as_slice()
        }
        crate::native_bridge::PendingHistoryTraversalAction::CrossDocument(traversal) => {
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
) -> bool {
    let results = traversal.results;
    let history = history_traversal_target_window(scope, host, traversal.target)
        .and_then(|window| window_history_for_holder(scope, window));
    let Some(history) = history else {
        let error = navigation_dom_exception(scope, "Navigation was canceled", "AbortError");
        reject_pending_navigation_results(scope, &results, error);
        return false;
    };
    if history_index(scope, history) != traversal.target_index {
        let target_entry = history_entries(scope, history)
            .and_then(|entries| entries.get_index(scope, traversal.target_index))
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
        let owner = runtime_window_owner(scope, history);
        let navigation = if navigation_document_has_opaque_origin(scope, owner) {
            None
        } else {
            window_navigation_for_holder(scope, owner)
        };
        let info = traversal
            .info
            .as_ref()
            .map(|info| v8::Local::new(scope, info));
        let outcome = navigation
            .map(|navigation| {
                dispatch_navigation_traverse_event_with_outcome(
                    scope,
                    navigation,
                    history,
                    traversal.target_index,
                    info,
                )
            })
            .unwrap_or_else(NavigationDispatchOutcome::proceed);
        let transition_resolver = outcome
            .precommit_event
            .and_then(|event| precommit_transition_resolver_from_event(scope, event));
        let owner_still_active = navigation_document_is_active(scope, owner);
        let target_still_available = target_entry_is_still_available(
            scope,
            history,
            traversal.target_index,
            target_entry,
            traversal.target_key.as_deref(),
        );
        if let Some(error) = outcome.abort_error {
            let _execution = ScriptExecutionScope::enter(scope);
            if let Some(navigation) = navigation {
                reject_traversal_transition_committed(
                    scope,
                    navigation,
                    transition_resolver,
                    error,
                );
                settle_navigation_transition_finished_local(
                    scope,
                    navigation,
                    transition_resolver,
                    Some(error),
                );
            }
            reject_pending_navigation_results(scope, &results, error);
            return false;
        }
        if let Some(error) = outcome.precommit_error {
            let _execution = ScriptExecutionScope::enter(scope);
            if let Some(navigation) = navigation {
                reject_traversal_transition_committed(
                    scope,
                    navigation,
                    transition_resolver,
                    error,
                );
                finish_navigation_error_events(scope, navigation, error, "");
                settle_navigation_transition_finished_local(
                    scope,
                    navigation,
                    transition_resolver,
                    Some(error),
                );
            }
            reject_pending_navigation_results(scope, &results, error);
            return false;
        }
        if !outcome.proceed || !owner_still_active || !target_still_available {
            let _execution = ScriptExecutionScope::enter(scope);
            let error = navigation_dom_exception(scope, "Navigation was canceled", "AbortError");
            if let Some(navigation) = navigation {
                reject_traversal_transition_committed(
                    scope,
                    navigation,
                    transition_resolver,
                    error,
                );
                settle_navigation_transition_finished_local(
                    scope,
                    navigation,
                    transition_resolver,
                    Some(error),
                );
            }
            mark_navigation_outcome_default_prevented(scope, &outcome);
            if let Some(signal) = outcome.signal
                && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
            {
                unsafe { &mut *host_ptr }.abort_signal(scope, signal, error);
            }
            if let Some(navigation) = navigation {
                let filename =
                    super::super::navigation_window::window_location_for_holder(scope, owner)
                        .and_then(|location| {
                            super::super::location_runtime::location_href_slot(scope, location)
                        })
                        .unwrap_or_default();
                if outcome.proceed && owner_still_active && !target_still_available {
                    reject_pending_navigation_results(scope, &results, error);
                    return false;
                }
                finish_navigation_error_events(scope, navigation, error, &filename);
            }
            reject_pending_navigation_results(scope, &results, error);
            return false;
        }
        if outcome.intercepted
            && let Some(navigation) = navigation
        {
            let transition_resolver = transition_resolver.or_else(|| {
                navigation_current_entry(scope, owner).and_then(|from| {
                    install_navigation_transition(
                        scope,
                        navigation,
                        from,
                        outcome.destination,
                        "traverse",
                    )
                })
            });
            if outcome.precommit_result.is_some() {
                let queued = queue_pending_precommit_history_traversal(
                    scope,
                    navigation,
                    history,
                    traversal.target_index,
                    outcome,
                    &results,
                    transition_resolver,
                );
                if !queued {
                    let _execution = ScriptExecutionScope::enter(scope);
                    let error =
                        navigation_dom_exception(scope, "Navigation was canceled", "AbortError");
                    reject_traversal_transition_committed(
                        scope,
                        navigation,
                        transition_resolver,
                        error,
                    );
                    finish_navigation_error_events(scope, navigation, error, "");
                    reject_pending_navigation_results(scope, &results, error);
                    settle_navigation_transition_finished_local(
                        scope,
                        navigation,
                        transition_resolver,
                        Some(error),
                    );
                }
                return queued;
            }
            let (committed_resolvers, finished_resolvers) =
                pending_result_resolver_arrays(scope, &results);
            commit_intercepted_history_traversal(
                scope,
                InterceptedHistoryTraversal {
                    navigation,
                    history,
                    target_index: traversal.target_index,
                    event: outcome.precommit_event,
                    signal: outcome.signal,
                    intercept_result: outcome.intercept_result,
                    committed_resolvers,
                    finished_resolvers,
                    transition_resolver,
                },
            );
            return true;
        }
        let pending_results = (!results.is_empty()).then_some(results.as_slice());
        apply_history_entry(
            scope,
            history,
            traversal.target_index,
            true,
            pending_results,
        );
        return true;
    }
    if traversal.target_key.is_some()
        && !target_entry_is_still_available(
            scope,
            history,
            traversal.target_index,
            None,
            traversal.target_key.as_deref(),
        )
    {
        let error = navigation_dom_exception(scope, "Navigation was canceled", "AbortError");
        reject_pending_navigation_results(scope, &results, error);
        return false;
    }
    let owner = runtime_window_owner(scope, history);
    let resolved_entry = navigation_current_entry(scope, owner)
        .map(v8::Local::<v8::Value>::from)
        .unwrap_or_else(|| v8::undefined(scope).into());
    resolve_pending_navigation_results(scope, results, resolved_entry);
    true
}

pub(in crate::context_bootstrap) fn history_traversal_target_window<'s>(
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

fn target_entry_is_still_available<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    target_index: u32,
    expected_entry: Option<v8::Local<'s, v8::Object>>,
    expected_key: Option<&str>,
) -> bool {
    let current_entry = history_entries(scope, history)
        .and_then(|entries| entries.get_index(scope, target_index))
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    if let Some(expected_key) = expected_key {
        return current_entry
            .and_then(|entry| navigation_entry_key_value(scope, entry))
            .is_some_and(|key| key == expected_key);
    }
    expected_entry.is_none_or(|expected_entry| {
        current_entry.is_some_and(|entry| entry.strict_equals(expected_entry.into()))
    })
}

fn queue_pending_precommit_history_traversal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    history: v8::Local<'s, v8::Object>,
    target_index: u32,
    outcome: NavigationDispatchOutcome<'s>,
    results: &[crate::native_bridge::PendingNavigationResult],
    transition_resolver: Option<v8::Local<'s, v8::PromiseResolver>>,
) -> bool {
    let Some(precommit_result) = outcome.precommit_result else {
        return false;
    };
    let Some(result_object) = v8::Local::<v8::Object>::try_from(precommit_result).ok() else {
        return false;
    };
    let Some(then) = result_object
        .get(scope, v8str(scope, "then").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return false;
    };
    let Some(event) = outcome.precommit_event else {
        return false;
    };
    let (committed_resolvers, finished_resolvers) = pending_result_resolver_arrays(scope, results);
    let promise = v8::Local::<v8::Promise>::try_from(precommit_result)
        .is_ok()
        .then_some(precommit_result);
    let data = TraversalPrecommitDataDeclaration {
        active: true,
        navigation,
        history,
        event,
        signal: outcome.signal,
        target_index,
        promise,
        committed_resolvers,
        finished_resolvers,
        transition_resolver,
    }
    .bind(scope)
    .expect("traversal precommit data should bind");
    set_navigation_pending_traversal_precommit(scope, navigation, data);
    let Some(on_fulfilled) = v8::Function::builder(traversal_precommit_fulfilled_callback)
        .data(data.into())
        .build(scope)
    else {
        return false;
    };
    let Some(on_rejected) = v8::Function::builder(traversal_precommit_rejected_callback)
        .data(data.into())
        .build(scope)
    else {
        return false;
    };
    then.call(
        scope,
        precommit_result,
        &[on_fulfilled.into(), on_rejected.into()],
    )
    .is_some()
}

fn pending_result_resolver_arrays<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    results: &[crate::native_bridge::PendingNavigationResult],
) -> (v8::Local<'s, v8::Array>, v8::Local<'s, v8::Array>) {
    let committed = v8::Array::new(scope, results.len() as i32);
    let finished = v8::Array::new(scope, results.len() as i32);
    for (index, result) in results.iter().enumerate() {
        let committed_resolver = v8::Local::new(scope, &result.committed_resolver);
        let finished_resolver = v8::Local::new(scope, &result.finished_resolver);
        let _ = committed.set_index(scope, index as u32, committed_resolver.into());
        let _ = finished.set_index(scope, index as u32, finished_resolver.into());
    }
    (committed, finished)
}

fn traversal_precommit_is_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> bool {
    v8::Local::<v8::Object>::try_from(data)
        .ok()
        .is_some_and(|data| traversal_private_bool(scope, data, TRAVERSAL_PRECOMMIT_ACTIVE_SLOT))
}

fn set_traversal_precommit_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Object>,
    active: bool,
) {
    set_private_value(
        scope,
        data,
        TRAVERSAL_PRECOMMIT_ACTIVE_SLOT,
        v8::Boolean::new(scope, active).into(),
    );
}

fn set_traversal_precommit_inactive<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Object>,
) {
    set_traversal_precommit_active(scope, data, false);
    let Some(navigation) = get_private_value(scope, data, TRAVERSAL_PRECOMMIT_NAVIGATION_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    clear_navigation_pending_traversal_precommit_if_current(scope, navigation, data);
}

#[allow(clippy::type_complexity)]
fn traversal_precommit_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<(
    v8::Local<'s, v8::Object>,
    v8::Local<'s, v8::Object>,
    v8::Local<'s, v8::Object>,
    Option<v8::Local<'s, v8::Object>>,
    u32,
    Option<v8::Local<'s, v8::Promise>>,
    v8::Local<'s, v8::Array>,
    v8::Local<'s, v8::Array>,
)> {
    let data = v8::Local::<v8::Object>::try_from(data).ok()?;
    let navigation = get_private_value(scope, data, TRAVERSAL_PRECOMMIT_NAVIGATION_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let history = get_private_value(scope, data, TRAVERSAL_PRECOMMIT_HISTORY_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let event = get_private_value(scope, data, TRAVERSAL_PRECOMMIT_EVENT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let signal = get_private_value(scope, data, TRAVERSAL_PRECOMMIT_SIGNAL_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let target_index = get_private_value(scope, data, TRAVERSAL_PRECOMMIT_TARGET_INDEX_SLOT)
        .and_then(|value| value.uint32_value(scope))?;
    let promise = get_private_value(scope, data, TRAVERSAL_PRECOMMIT_PROMISE_SLOT)
        .and_then(|value| v8::Local::<v8::Promise>::try_from(value).ok());
    let committed_resolvers =
        get_private_value(scope, data, TRAVERSAL_PRECOMMIT_COMMITTED_RESOLVERS_SLOT)
            .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())?;
    let finished_resolvers =
        get_private_value(scope, data, TRAVERSAL_PRECOMMIT_FINISHED_RESOLVERS_SLOT)
            .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())?;
    Some((
        navigation,
        history,
        event,
        signal,
        target_index,
        promise,
        committed_resolvers,
        finished_resolvers,
    ))
}

pub(in crate::context_bootstrap) fn cancel_pending_precommit_history_traversal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(data) = navigation_pending_traversal_precommit(scope, navigation)
        .filter(|data| traversal_precommit_is_active(scope, (*data).into()))
    else {
        return false;
    };
    let Some((
        navigation,
        history,
        _,
        signal,
        target_index,
        _,
        committed_resolvers,
        finished_resolvers,
    )) = traversal_precommit_data(scope, data.into())
    else {
        return false;
    };
    set_traversal_precommit_inactive(scope, data);
    super::super::joint_history::cancel_precommit(scope, history, target_index);
    let error =
        navigation_dom_exception(scope, "Navigation was canceled before commit", "AbortError");
    let _execution = ScriptExecutionScope::enter(scope);
    let transition_resolver = traversal_transition_resolver(scope, data.into());
    let committed_resolver = transition_resolver
        .filter(|resolver| navigation_transition_matches_resolver(scope, navigation, *resolver))
        .and_then(|_| take_navigation_transition_committed_resolver(scope, navigation));
    if let Some(signal) = signal
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        unsafe { &mut *host_ptr }.abort_signal(scope, signal, error);
    }
    reject_resolver_array(scope, committed_resolvers, error, false);
    reject_resolver_array(scope, finished_resolvers, error, true);
    finish_navigation_error_events(scope, navigation, error, "");
    if let Some(resolver) = committed_resolver {
        let _ = resolver.reject(scope, error);
    }
    settle_navigation_transition_finished_local(
        scope,
        navigation,
        transition_resolver,
        Some(error),
    );
    true
}

fn traversal_precommit_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !traversal_precommit_is_active(scope, args.data()) {
        return;
    }
    let Some((
        navigation,
        history,
        event,
        signal,
        target_index,
        _,
        committed_resolvers,
        finished_resolvers,
    )) = traversal_precommit_data(scope, args.data())
    else {
        return;
    };
    if let Ok(data) = v8::Local::<v8::Object>::try_from(args.data()) {
        set_traversal_precommit_inactive(scope, data);
    }
    let transition_resolver = traversal_transition_resolver(scope, args.data());
    commit_intercepted_history_traversal(
        scope,
        InterceptedHistoryTraversal {
            navigation,
            history,
            target_index,
            event: Some(event),
            signal,
            intercept_result: None,
            committed_resolvers,
            finished_resolvers,
            transition_resolver,
        },
    );
}

fn traversal_precommit_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !traversal_precommit_is_active(scope, args.data()) {
        return;
    }
    let Some((
        navigation,
        history,
        _,
        signal,
        target_index,
        promise,
        committed_resolvers,
        finished_resolvers,
    )) = traversal_precommit_data(scope, args.data())
    else {
        return;
    };
    if let Ok(data) = v8::Local::<v8::Object>::try_from(args.data()) {
        set_traversal_precommit_inactive(scope, data);
    }
    super::super::joint_history::cancel_precommit(scope, history, target_index);
    let error = promise
        .filter(|promise| promise.state() == v8::PromiseState::Rejected)
        .map(|promise| promise.result(scope))
        .unwrap_or_else(|| args.get(0));
    let _execution = ScriptExecutionScope::enter(scope);
    let transition_resolver = traversal_transition_resolver(scope, args.data());
    let committed_resolver = transition_resolver
        .filter(|resolver| navigation_transition_matches_resolver(scope, navigation, *resolver))
        .and_then(|_| take_navigation_transition_committed_resolver(scope, navigation));
    if let Some(signal) = signal
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        unsafe { &mut *host_ptr }.abort_signal(scope, signal, error);
    }
    reject_resolver_array(scope, committed_resolvers, error, false);
    reject_resolver_array(scope, finished_resolvers, error, true);
    finish_navigation_error_events(scope, navigation, error, "");
    if let Some(resolver) = committed_resolver {
        let _ = resolver.reject(scope, error);
    }
    settle_navigation_transition_finished_local(
        scope,
        navigation,
        transition_resolver,
        Some(error),
    );
}

fn commit_intercepted_history_traversal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: InterceptedHistoryTraversal<'s>,
) {
    // HTML's "prepare to run script" keeps commit-time Promise reactions after
    // currententrychange listeners and intercept handlers, even on a native task.
    let execution = ScriptExecutionScope::enter(scope);
    if let Some(event) = data.event {
        finish_navigation_precommit(scope, event);
    }
    let Some(applied) =
        apply_history_entry_commit(scope, data.history, data.target_index, Some("other"))
    else {
        super::super::joint_history::cancel_precommit(scope, data.history, data.target_index);
        let error = navigation_dom_exception(scope, "Navigation was canceled", "AbortError");
        reject_traversal_transition_committed(
            scope,
            data.navigation,
            data.transition_resolver,
            error,
        );
        finish_navigation_error_events(scope, data.navigation, error, "");
        reject_resolver_array(scope, data.committed_resolvers, error, false);
        reject_resolver_array(scope, data.finished_resolvers, error, true);
        settle_navigation_transition_finished_local(
            scope,
            data.navigation,
            data.transition_resolver,
            Some(error),
        );
        return;
    };
    let url = v8_string(scope, &applied.url).unwrap_or_else(|| v8::String::empty(scope));
    let active_intercept = TraversalInterceptSettlementDataDeclaration {
        active: true,
        navigation: data.navigation,
        signal: data.signal,
        finished_resolvers: data.finished_resolvers,
        value: applied.resolved_entry,
        url,
        transition_resolver: data.transition_resolver,
    }
    .bind(scope)
    .expect("traversal intercept settlement data should bind");
    set_navigation_active_traversal_intercept(scope, data.navigation, active_intercept);
    resolve_resolver_array(scope, data.committed_resolvers, applied.resolved_entry);
    resolve_navigation_transition_committed(scope, data.navigation);
    dispatch_history_entry_currententrychange(scope, &applied);
    let (intercept_error, intercept_result) = if let Some(event) = data.event {
        run_navigation_precommit_deferred_handlers(scope, event)
    } else {
        (None, data.intercept_result)
    };
    suppress_intercept_result_unhandled_rejection(scope, intercept_result);
    drop(execution);
    dispatch_history_entry_post_commit_events(scope, &applied, true);
    if !traversal_intercept_is_active(scope, active_intercept.into()) {
        return;
    }
    if let Some(error) = intercept_error {
        finish_traversal_intercept(scope, active_intercept.into(), Some(error));
    } else {
        // Waiting for an empty handler list is asynchronous too. In particular,
        // a precommit Promise callback must not finish before committed reactions.
        let result = intercept_result.or_else(|| {
            let resolver = v8::PromiseResolver::new(scope)?;
            resolver.resolve(scope, v8::undefined(scope).into())?;
            Some(resolver.get_promise(scope).into())
        });
        if result.is_none_or(|result| {
            !queue_pending_traversal_intercept_settlement(scope, active_intercept, result)
        }) {
            finish_traversal_intercept(scope, active_intercept.into(), None);
        }
    }
}

fn reject_traversal_transition_committed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    transition_resolver: Option<v8::Local<'s, v8::PromiseResolver>>,
    error: v8::Local<'s, v8::Value>,
) {
    if transition_resolver
        .is_some_and(|resolver| navigation_transition_matches_resolver(scope, navigation, resolver))
    {
        reject_navigation_transition_committed(scope, navigation, error);
    }
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
    data: v8::Local<'s, v8::Object>,
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
    let error = navigation_dom_exception(scope, "Navigation was canceled", "AbortError");
    finish_traversal_intercept(scope, data.into(), Some(error));
    true
}

fn traversal_intercept_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    finish_traversal_intercept(scope, args.data(), None);
}

fn traversal_intercept_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((_, _, _, _, _, promise)) = traversal_intercept_data(scope, args.data()) else {
        return;
    };
    let error = promise
        .filter(|promise| promise.state() == v8::PromiseState::Rejected)
        .map(|promise| promise.result(scope))
        .unwrap_or_else(|| args.get(0));
    finish_traversal_intercept(scope, args.data(), Some(error));
}

fn finish_traversal_intercept<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
    error: Option<v8::Local<'s, v8::Value>>,
) {
    let Some((navigation, signal, finished_resolvers, resolved_value, url, _)) =
        traversal_intercept_data(scope, data)
    else {
        return;
    };
    let _execution = ScriptExecutionScope::enter(scope);
    let transition_resolver = traversal_transition_resolver(scope, data);
    set_traversal_intercept_inactive(scope, navigation, data);
    let owner = runtime_window_owner(scope, navigation);
    let active = navigation_document_is_active(scope, owner);
    let error = error.or_else(|| {
        (!active).then(|| navigation_dom_exception(scope, "Navigation was canceled", "AbortError"))
    });
    if let Some(error) = error {
        if let Some(signal) = signal
            && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
        {
            unsafe { &mut *host_ptr }.abort_signal(scope, signal, error);
        }
        let filename = if active {
            url
        } else {
            let top_owner = runtime_top_window_owner(scope, owner);
            window_location_for_holder(scope, top_owner)
                .and_then(|location| {
                    super::super::location_runtime::location_href_slot(scope, location)
                })
                .unwrap_or(url)
        };
        finish_navigation_error_events(scope, navigation, error, &filename);
        reject_resolver_array(scope, finished_resolvers, error, true);
    } else {
        perform_navigation_scroll_if_needed(scope, navigation, &url, true);
        dispatch_navigation_success(scope, navigation);
        resolve_resolver_array(scope, finished_resolvers, resolved_value);
    }
    settle_navigation_transition_finished_local(scope, navigation, transition_resolver, error);
}

fn resolve_resolver_array<'s>(
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

fn reject_resolver_array<'s>(
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
