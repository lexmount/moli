use super::history_runtime::cancel_active_history_traversal_intercept_settlement;
use super::navigation_entry::navigation_current_entry;
use super::navigation_lifecycle::{
    complete_navigation_attempt, enqueue_navigation_lifecycle_microtask,
    finish_navigation_error_events, finish_navigation_success_events, navigation_attempt_is_active,
    settle_navigation_committed, settle_navigation_finished_rejected,
    settle_navigation_finished_resolved_after_reactions,
    settle_navigation_finished_resolved_immediately, settle_navigation_transition_finished,
};
pub(super) use super::navigation_lifecycle::{
    perform_navigation_scroll_if_needed, reset_navigation_focus_if_unchanged,
};
use super::*;
use crate::util::{get_private_value, set_private_value};
use moli_webapi_declare::WebApiObject;

const CROSS_DOCUMENT_PENDING_ACTIVE_SLOT: &str = "__lmCrossDocumentPendingActive";
const CROSS_DOCUMENT_PENDING_SIGNAL_SLOT: &str = "__lmCrossDocumentPendingSignal";
const CROSS_DOCUMENT_PENDING_COMMITTED_REJECT_SLOT: &str =
    "__lmCrossDocumentPendingCommittedReject";
const CROSS_DOCUMENT_PENDING_FINISHED_REJECT_SLOT: &str = "__lmCrossDocumentPendingFinishedReject";
const CROSS_DOCUMENT_PENDING_HREF_SLOT: &str = "__lmCrossDocumentPendingHref";
const NAVIGATION_ACTIVE_CROSS_DOCUMENT_PENDING_SLOT: &str =
    "__lmNavigationActiveCrossDocumentPending";

#[derive(WebApiObject)]
#[webapi(plain)]
struct NavigationResultDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    committed: v8::Local<'scope, v8::Promise>,
    #[webapi(data_property, enumerable)]
    finished: v8::Local<'scope, v8::Promise>,
}

#[derive(Default, WebApiObject)]
#[webapi(plain)]
struct NavigationResultFallbackDeclaration {
    #[webapi(data_property, enumerable, name = "committed", init = "undefined")]
    committed: (),
    #[webapi(data_property, enumerable, name = "finished", init = "undefined")]
    finished: (),
}

#[derive(Default, WebApiObject)]
#[webapi(plain)]
struct CapturedPromiseResolverRecordDeclaration {
    #[webapi(data_property = "resolve", init = "undefined")]
    resolve: (),
    #[webapi(data_property = "reject", init = "undefined")]
    reject: (),
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct CapturedPromiseResolverFunctionsDeclaration<'scope> {
    #[webapi(data_property)]
    resolve: v8::Local<'scope, v8::Value>,
    #[webapi(data_property)]
    reject: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct CrossDocumentPendingNavigationDeclaration<'scope> {
    #[webapi(
        slot = CROSS_DOCUMENT_PENDING_ACTIVE_SLOT,
        constructor_default = true
    )]
    active: bool,
    #[webapi(slot = CROSS_DOCUMENT_PENDING_SIGNAL_SLOT)]
    signal: Option<v8::Local<'scope, v8::Object>>,
    #[webapi(slot = CROSS_DOCUMENT_PENDING_COMMITTED_REJECT_SLOT)]
    committed_reject: v8::Local<'scope, v8::Function>,
    #[webapi(slot = CROSS_DOCUMENT_PENDING_FINISHED_REJECT_SLOT)]
    finished_reject: v8::Local<'scope, v8::Function>,
    #[webapi(slot = CROSS_DOCUMENT_PENDING_HREF_SLOT)]
    href: v8::Local<'scope, v8::Value>,
}

fn navigation_active_cross_document_pending<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(
        scope,
        navigation,
        NAVIGATION_ACTIVE_CROSS_DOCUMENT_PENDING_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn set_navigation_active_cross_document_pending<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    data: v8::Local<'s, v8::Object>,
) {
    set_private_value(
        scope,
        navigation,
        NAVIGATION_ACTIVE_CROSS_DOCUMENT_PENDING_SLOT,
        data.into(),
    );
}

pub(super) fn navigation_rejected_invalid_state_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
) -> v8::Local<'s, v8::Object> {
    navigation_rejected_dom_exception_result(scope, message, "InvalidStateError")
}

pub(super) fn navigation_rejected_dom_exception_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
    name: &str,
) -> v8::Local<'s, v8::Object> {
    let rejected_value = navigation_dom_exception(scope, message, name);
    navigation_rejected_value_result(scope, rejected_value)
}

pub(super) fn navigation_rejected_value_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rejected_value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Object> {
    let Some(committed_resolver) = v8::PromiseResolver::new(scope) else {
        return navigation_result_fallback_object(scope);
    };
    let Some(finished_resolver) = v8::PromiseResolver::new(scope) else {
        return navigation_result_fallback_object(scope);
    };
    let committed = committed_resolver.get_promise(scope);
    let finished = finished_resolver.get_promise(scope);
    let _ = committed_resolver.reject(scope, rejected_value);
    let _ = finished_resolver.reject(scope, rejected_value);
    suppress_unhandled_rejection(scope, finished);
    navigation_result_object(scope, committed, finished)
}

pub(super) fn navigation_dom_exception<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
    name: &str,
) -> v8::Local<'s, v8::Value> {
    crate::context_bootstrap::new_dom_exception_value(scope, message, name)
}

pub(super) fn navigation_pending_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    let Some(committed_resolver) = v8::PromiseResolver::new(scope) else {
        return navigation_result_fallback_object(scope);
    };
    let Some(finished_resolver) = v8::PromiseResolver::new(scope) else {
        return navigation_result_fallback_object(scope);
    };
    let committed = committed_resolver.get_promise(scope);
    let finished = finished_resolver.get_promise(scope);
    navigation_result_object(scope, committed, finished)
}

pub(super) fn navigation_cross_document_pending_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    signal: Option<v8::Local<'s, v8::Object>>,
    href: &str,
) -> v8::Local<'s, v8::Object> {
    let Some((committed, _, committed_reject)) = captured_promise(scope) else {
        return navigation_pending_result(scope);
    };
    let Some((finished, _, finished_reject)) = captured_promise(scope) else {
        return navigation_pending_result(scope);
    };
    suppress_unhandled_rejection(scope, finished);
    let href = v8_string(scope, href)
        .map(v8::Local::<v8::Value>::from)
        .unwrap_or_else(|| v8::String::empty(scope).into());
    let data = CrossDocumentPendingNavigationDeclaration::new(
        signal,
        committed_reject,
        finished_reject,
        href,
    )
    .bind(scope)
    .expect("cross-document pending navigation declaration should bind");
    set_navigation_active_cross_document_pending(scope, navigation, data);
    navigation_result_object(scope, committed, finished)
}

pub(super) fn navigation_immediate_current_entry_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let resolved_value = navigation_current_entry(scope, owner)
        .map(v8::Local::<v8::Value>::from)
        .unwrap_or_else(|| v8::undefined(scope).into());
    navigation_immediate_result_with_value(scope, resolved_value)
}

pub(super) fn navigation_current_entry_result_with_deferred_finished<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    navigation: v8::Local<'s, v8::Object>,
    signal: Option<v8::Local<'s, v8::Object>>,
    href: &str,
) -> v8::Local<'s, v8::Object> {
    let resolved_value = navigation_current_entry(scope, owner)
        .map(v8::Local::<v8::Value>::from)
        .unwrap_or_else(|| v8::undefined(scope).into());
    navigation_result_with_deferred_finished(scope, navigation, signal, resolved_value, href)
}

pub(super) fn navigation_current_entry_result_with_task_finished<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    navigation: v8::Local<'s, v8::Object>,
    signal: Option<v8::Local<'s, v8::Object>>,
    href: &str,
) -> v8::Local<'s, v8::Object> {
    let resolved_value = navigation_current_entry(scope, owner)
        .map(v8::Local::<v8::Value>::from)
        .unwrap_or_else(|| v8::undefined(scope).into());
    navigation_result_with_task_finished(scope, navigation, signal, resolved_value, href)
}

pub(super) fn queue_same_document_navigation_success<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    signal: Option<v8::Local<'s, v8::Object>>,
    href: &str,
) {
    queue_same_document_navigation_finished(
        scope, navigation, signal, None, None, None, None, None, href,
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn queue_same_document_navigation_finished<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    signal: Option<v8::Local<'s, v8::Object>>,
    committed_resolve: Option<v8::Local<'s, v8::Function>>,
    finished_resolve: Option<v8::Local<'s, v8::Function>>,
    finished_reject: Option<v8::Local<'s, v8::Function>>,
    resolved_value: Option<v8::Local<'s, v8::Value>>,
    transition_resolver: Option<v8::Local<'s, v8::PromiseResolver>>,
    href: &str,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        finish_navigation_success_events(scope, navigation, href);
        if let Some(value) = resolved_value {
            if let Some(resolve) = committed_resolve {
                settle_navigation_committed(scope, navigation, resolve, value);
            }
            if let Some(resolve) = finished_resolve {
                settle_navigation_finished_resolved_immediately(scope, resolve, value);
            }
        }
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let attempt_id = host.begin_navigation_lifecycle_attempt("same-document-finished");
    let resolve_committed_after_success_is_scheduled = committed_resolve;
    if host.queue_microtask_navigation_finished_result(
        scope,
        attempt_id,
        navigation,
        signal,
        None,
        finished_resolve,
        finished_reject,
        resolved_value,
        transition_resolver,
        href,
    ) && let Some(callback) =
        v8::Function::builder(microtask_navigation_finished_flush_callback).build(scope)
    {
        enqueue_navigation_lifecycle_microtask(scope, callback);
    }
    if let (Some(resolve), Some(value)) =
        (resolve_committed_after_success_is_scheduled, resolved_value)
    {
        reset_navigation_focus_if_unchanged(scope, navigation);
        settle_navigation_committed(scope, navigation, resolve, value);
    }
}

pub(super) fn cancel_pending_same_document_navigation_finishes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> bool {
    let mut canceled = cancel_active_history_traversal_intercept_settlement(scope, navigation);
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return canceled;
    };
    let host = unsafe { &mut *host_ptr };
    let pending = host.take_pending_navigation_finished_results_for_navigation(scope, navigation);
    if pending.is_empty() {
        return canceled;
    }
    canceled = true;
    for result in pending {
        host.cancel_navigation_lifecycle_attempt(result.attempt_id);
        let error = navigation_dom_exception(scope, "Navigation was canceled", "AbortError");
        if let Some(signal) = result.signal {
            let signal = v8::Local::new(scope, signal);
            host.abort_signal(scope, signal, error);
        }
        finish_navigation_error_events(scope, navigation, error, &result.href);
        if let (Some(resolve), Some(value)) = (result.committed_resolve, result.resolved_value) {
            let resolve = v8::Local::new(scope, resolve);
            let value = v8::Local::new(scope, value);
            settle_navigation_committed(scope, navigation, resolve, value);
        }
        if let Some(reject) = result.finished_reject {
            let reject = v8::Local::new(scope, reject);
            settle_navigation_finished_rejected(scope, reject, error);
        }
        settle_navigation_transition_finished(
            scope,
            navigation,
            result.transition_resolver,
            Some(error),
        );
    }
    canceled
}

pub(super) fn cancel_active_cross_document_navigation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    next_href: Option<&str>,
) -> bool {
    let Some(data) = navigation_active_cross_document_pending(scope, navigation) else {
        return false;
    };
    if !get_private_value(scope, data, CROSS_DOCUMENT_PENDING_ACTIVE_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
    {
        return false;
    }
    let href = get_private_value(scope, data, CROSS_DOCUMENT_PENDING_HREF_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    if next_href.is_some_and(|next_href| next_href == href) {
        return false;
    }
    set_private_value(
        scope,
        data,
        CROSS_DOCUMENT_PENDING_ACTIVE_SLOT,
        v8::Boolean::new(scope, false).into(),
    );
    clear_active_cross_document_navigation(scope, navigation);
    let owner = super::navigation_window::runtime_window_owner(scope, navigation);
    super::navigation_cancellation::clear_pending_cross_document_navigation_for_window(
        scope, owner,
    );
    let error = navigation_dom_exception(scope, "Navigation was canceled", "AbortError");
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let host = unsafe { &mut *host_ptr };
        if let Some(signal) = get_private_value(scope, data, CROSS_DOCUMENT_PENDING_SIGNAL_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        {
            host.abort_signal(scope, signal, error);
        }
    }
    finish_navigation_error_events(scope, navigation, error, &href);
    let receiver = v8::undefined(scope).into();
    if let Some(reject) =
        get_private_value(scope, data, CROSS_DOCUMENT_PENDING_COMMITTED_REJECT_SLOT)
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        let _ = reject.call(scope, receiver, &[error]);
    }
    if let Some(reject) =
        get_private_value(scope, data, CROSS_DOCUMENT_PENDING_FINISHED_REJECT_SLOT)
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        let _ = reject.call(scope, receiver, &[error]);
    }
    true
}

pub(super) fn clear_active_cross_document_navigation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) {
    set_private_value(
        scope,
        navigation,
        NAVIGATION_ACTIVE_CROSS_DOCUMENT_PENDING_SLOT,
        v8::undefined(scope).into(),
    );
}

pub(super) fn clear_active_cross_document_navigation_if_matches<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    href: &str,
) {
    let Some(data) = navigation_active_cross_document_pending(scope, navigation) else {
        return;
    };
    if !get_private_value(scope, data, CROSS_DOCUMENT_PENDING_ACTIVE_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
    {
        return;
    }
    let active_href = get_private_value(scope, data, CROSS_DOCUMENT_PENDING_HREF_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope));
    if active_href.as_deref() == Some(href) {
        clear_active_cross_document_navigation(scope, navigation);
    }
}

pub(super) fn cancel_pending_same_document_navigation_finishes_including_reentrant<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> bool {
    let mut canceled = false;
    for _ in 0..16 {
        if !cancel_pending_same_document_navigation_finishes(scope, navigation) {
            return canceled;
        }
        canceled = true;
    }
    canceled
}

pub(super) struct NavigationPendingFinishedResult<'s> {
    pub(super) object: v8::Local<'s, v8::Object>,
    pub(super) committed_resolve: v8::Local<'s, v8::Function>,
    pub(super) finished_resolve: v8::Local<'s, v8::Function>,
    pub(super) finished_reject: v8::Local<'s, v8::Function>,
    pub(super) resolved_value: v8::Local<'s, v8::Value>,
}

pub(super) struct NavigationPendingCommitResult<'s> {
    pub(super) object: v8::Local<'s, v8::Object>,
    pub(super) committed_resolve: v8::Local<'s, v8::Function>,
    pub(super) committed_reject: v8::Local<'s, v8::Function>,
    pub(super) finished_resolve: v8::Local<'s, v8::Function>,
    pub(super) finished_reject: v8::Local<'s, v8::Function>,
}

pub(super) fn navigation_current_entry_result_with_pending_finished<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<NavigationPendingFinishedResult<'s>> {
    let resolved_value = navigation_current_entry(scope, owner)
        .map(v8::Local::<v8::Value>::from)
        .unwrap_or_else(|| v8::undefined(scope).into());
    navigation_result_with_pending_finished(scope, resolved_value)
}

pub(super) fn navigation_result_with_pending_commit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<NavigationPendingCommitResult<'s>> {
    let (committed, committed_resolve, committed_reject) = captured_promise(scope)?;
    let (finished, finished_resolve, finished_reject) = captured_promise(scope)?;
    let object = navigation_result_object(scope, committed, finished);
    Some(NavigationPendingCommitResult {
        object,
        committed_resolve,
        committed_reject,
        finished_resolve,
        finished_reject,
    })
}

pub(super) fn navigation_immediate_result_with_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolved_value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Object> {
    let Some(committed_resolver) = v8::PromiseResolver::new(scope) else {
        return navigation_result_fallback_object(scope);
    };
    let Some(finished_resolver) = v8::PromiseResolver::new(scope) else {
        return navigation_result_fallback_object(scope);
    };
    let committed = committed_resolver.get_promise(scope);
    let finished = finished_resolver.get_promise(scope);
    let _ = committed_resolver.resolve(scope, resolved_value);
    let _ = finished_resolver.resolve(scope, resolved_value);
    navigation_result_object(scope, committed, finished)
}

fn navigation_result_with_pending_finished<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolved_value: v8::Local<'s, v8::Value>,
) -> Option<NavigationPendingFinishedResult<'s>> {
    let (committed, committed_resolve, _) = captured_promise(scope)?;
    let (finished, finished_resolve, finished_reject) = captured_promise(scope)?;
    suppress_unhandled_rejection(scope, finished);
    let object = navigation_result_object(scope, committed, finished);
    Some(NavigationPendingFinishedResult {
        object,
        committed_resolve,
        finished_resolve,
        finished_reject,
        resolved_value,
    })
}

fn captured_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<(
    v8::Local<'s, v8::Promise>,
    v8::Local<'s, v8::Function>,
    v8::Local<'s, v8::Function>,
)> {
    let captured = CapturedPromiseResolverRecordDeclaration::default()
        .bind(scope)
        .expect("captured promise resolver record declaration should bind");
    let executor = v8::Function::builder(capture_promise_resolver_functions_callback)
        .data(captured.into())
        .build(scope)?;
    let global = scope.get_current_context().global(scope);
    let promise_ctor = global
        .get(scope, v8str(scope, "Promise").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let promise = promise_ctor
        .new_instance(scope, &[executor.into()])
        .and_then(|value| v8::Local::<v8::Promise>::try_from(value).ok())?;
    let resolve = captured
        .get(scope, v8str(scope, "resolve").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let reject = captured
        .get(scope, v8str(scope, "reject").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    Some((promise, resolve, reject))
}

fn capture_promise_resolver_functions_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(captured) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    CapturedPromiseResolverFunctionsDeclaration::new(args.get(0), args.get(1))
        .initialize(scope, captured)
        .expect("captured promise resolver functions declaration should initialize");
}

fn navigation_result_with_deferred_finished<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    signal: Option<v8::Local<'s, v8::Object>>,
    resolved_value: v8::Local<'s, v8::Value>,
    href: &str,
) -> v8::Local<'s, v8::Object> {
    let Some((committed, committed_resolve, _)) = captured_promise(scope) else {
        return navigation_result_fallback_object(scope);
    };
    let Some((finished, finished_resolve, finished_reject)) = captured_promise(scope) else {
        return navigation_result_fallback_object(scope);
    };
    suppress_unhandled_rejection(scope, finished);
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        settle_navigation_committed(scope, navigation, committed_resolve, resolved_value);
        settle_navigation_finished_resolved_immediately(scope, finished_resolve, resolved_value);
        return navigation_result_object(scope, committed, finished);
    };
    let host = unsafe { &mut *host_ptr };
    let attempt_id = host.begin_navigation_lifecycle_attempt("deferred-finished");
    if host.queue_microtask_navigation_finished_result(
        scope,
        attempt_id,
        navigation,
        signal,
        None,
        Some(finished_resolve),
        Some(finished_reject),
        Some(resolved_value),
        None,
        href,
    ) && let Some(callback) =
        v8::Function::builder(microtask_navigation_finished_flush_callback).build(scope)
    {
        enqueue_navigation_lifecycle_microtask(scope, callback);
    }
    settle_navigation_committed(scope, navigation, committed_resolve, resolved_value);
    navigation_result_object(scope, committed, finished)
}

fn navigation_result_with_task_finished<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    signal: Option<v8::Local<'s, v8::Object>>,
    resolved_value: v8::Local<'s, v8::Value>,
    href: &str,
) -> v8::Local<'s, v8::Object> {
    let Some((committed, committed_resolve, _)) = captured_promise(scope) else {
        return navigation_result_fallback_object(scope);
    };
    let Some((finished, finished_resolve, finished_reject)) = captured_promise(scope) else {
        return navigation_result_fallback_object(scope);
    };
    suppress_unhandled_rejection(scope, finished);
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        settle_navigation_committed(scope, navigation, committed_resolve, resolved_value);
        settle_navigation_finished_resolved_immediately(scope, finished_resolve, resolved_value);
        return navigation_result_object(scope, committed, finished);
    };
    let host = unsafe { &mut *host_ptr };
    let attempt_id = host.begin_navigation_lifecycle_attempt("task-finished");
    let producer = host.queue_navigation_api_finished_task(
        scope,
        attempt_id,
        navigation,
        signal,
        finished_resolve,
        finished_reject,
        resolved_value,
        href,
    );
    if let Some(producer) = producer {
        let task_id = producer.task_id();
        if producer.send().is_err() {
            let task = host
                .take_pending_navigation_api_task(task_id)
                .expect("a rejected Navigation API route must retain its Host-local payload");
            reject_pending_navigation_api_task(scope, host, task.action);
        }
    } else {
        host.cancel_navigation_lifecycle_attempt(attempt_id);
        let error = navigation_dom_exception(scope, "Navigation was canceled", "AbortError");
        if let Some(signal) = signal {
            host.abort_signal(scope, signal, error);
        }
        finish_navigation_error_events(scope, navigation, error, href);
        settle_navigation_finished_rejected(scope, finished_reject, error);
    }
    settle_navigation_committed(scope, navigation, committed_resolve, resolved_value);
    navigation_result_object(scope, committed, finished)
}

fn microtask_navigation_finished_flush_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let pending = host.take_pending_microtask_navigation_finished_results();
    for result in pending {
        let _ = apply_pending_navigation_finished_result(scope, result);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NavigationFinishedResultApplication {
    /// The attempt was still authoritative, so success events and Promise
    /// settlement were applied to its Navigation object.
    Applied,
    /// A later cancellation/replacement already retired the attempt. The
    /// queued result owns no callback-visible application.
    IgnoredInactiveAttempt,
}

pub(crate) fn apply_pending_navigation_finished_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: crate::native_bridge::PendingNavigationFinishedResult,
) -> NavigationFinishedResultApplication {
    if !navigation_attempt_is_active(scope, result.attempt_id) {
        return NavigationFinishedResultApplication::IgnoredInactiveAttempt;
    }
    let navigation = v8::Local::new(scope, result.navigation);
    finish_navigation_success_events(scope, navigation, &result.href);
    let resolved_value = result
        .resolved_value
        .map(|value| v8::Local::new(scope, value));
    if let (Some(resolve), Some(value)) = (result.committed_resolve, resolved_value) {
        let resolve = v8::Local::new(scope, resolve);
        settle_navigation_committed(scope, navigation, resolve, value);
    }
    if let (Some(resolve), Some(value)) = (result.finished_resolve, resolved_value) {
        let resolve = v8::Local::new(scope, resolve);
        settle_navigation_finished_resolved_after_reactions(scope, resolve, value);
        settle_navigation_transition_finished(scope, navigation, result.transition_resolver, None);
    }
    complete_navigation_attempt(scope, result.attempt_id);
    NavigationFinishedResultApplication::Applied
}

fn reject_pending_navigation_api_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &mut JsContextHost,
    action: crate::native_bridge::PendingNavigationApiTaskAction,
) {
    match action {
        crate::native_bridge::PendingNavigationApiTaskAction::FinishResult(result) => {
            host.cancel_navigation_lifecycle_attempt(result.attempt_id);
            let navigation = v8::Local::new(scope, result.navigation);
            let error = navigation_dom_exception(scope, "Navigation was canceled", "AbortError");
            if let Some(signal) = result.signal {
                let signal = v8::Local::new(scope, signal);
                host.abort_signal(scope, signal, error);
            }
            finish_navigation_error_events(scope, navigation, error, &result.href);
            if let Some(reject) = result.finished_reject {
                let reject = v8::Local::new(scope, reject);
                settle_navigation_finished_rejected(scope, reject, error);
            }
            settle_navigation_transition_finished(
                scope,
                navigation,
                result.transition_resolver,
                Some(error),
            );
        }
    }
}

fn navigation_result_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    committed: v8::Local<'s, v8::Promise>,
    finished: v8::Local<'s, v8::Promise>,
) -> v8::Local<'s, v8::Object> {
    NavigationResultDeclaration::new(committed, finished)
        .bind(scope)
        .expect("NavigationResult declaration should bind")
}

fn navigation_result_fallback_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    NavigationResultFallbackDeclaration::default()
        .bind(scope)
        .expect("NavigationResult fallback declaration should bind")
}

pub(super) fn suppress_unhandled_rejection(
    scope: &mut v8::PinScope<'_, '_>,
    promise: v8::Local<'_, v8::Promise>,
) {
    let Some(noop) = v8::Function::builder(noop_rejection_handler).build(scope) else {
        return;
    };
    let Some(catch) = promise
        .get(scope, v8str(scope, "catch").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return;
    };
    let _ = catch.call(scope, promise.into(), &[noop.into()]);
}

fn noop_rejection_handler<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
}
