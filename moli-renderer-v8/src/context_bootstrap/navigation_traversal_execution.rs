use super::history_runtime::{
    apply_history_entry, history_traversal_target_window, reject_pending_navigation_results,
    route_history_traversal_task,
};
use super::navigation_entry::{history_entries, history_index, navigation_entry_key_value};
use super::navigation_events::{
    NavigationDispatchOutcome, dispatch_beforeunload_for_runtime_owner,
    dispatch_navigation_traverse_event, dispatch_navigation_traverse_event_with_outcome,
};
use super::navigation_result::{
    cancel_active_cross_document_navigation, navigation_dom_exception,
    navigation_immediate_current_entry_result, navigation_pending_result,
    navigation_rejected_dom_exception_result, track_cross_document_traversal_navigation,
};
use super::navigation_seed::history_entry_seed_for_traversal;
use super::navigation_window::{navigation_document_is_active, window_navigation_for_holder};
use super::navigation_window::{
    runtime_window_is_global, runtime_window_owner, window_history_for_holder,
    window_task_target_for_runtime_owner,
};
use super::*;
use crate::document_runtime::DomHandle;
use crate::native_bridge::{
    NavigationHistoryEntrySeed, PendingCrossDocumentTraversal, PendingHistoryTraversal,
    PendingHistoryTraversalAction, PendingNavigationResult, WindowTaskTarget,
};

pub(super) struct TraversalTarget<'s> {
    pub(super) owner: v8::Local<'s, v8::Object>,
    pub(super) history: v8::Local<'s, v8::Object>,
    pub(super) current_index: u32,
    pub(super) target_index: u32,
}

pub(super) fn queue_navigation_traversal_with_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: TraversalTarget<'s>,
    info: Option<v8::Local<'s, v8::Value>>,
) -> Option<v8::Local<'s, v8::Object>> {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        if !dispatch_traverse_event(scope, &target) {
            return Some(navigation_rejected_dom_exception_result(
                scope,
                "Navigation was canceled",
                "AbortError",
            ));
        }
        apply_history_entry(scope, target.history, target.target_index, true, None);
        return Some(navigation_immediate_current_entry_result(
            scope,
            target.owner,
        ));
    };
    let host = unsafe { &mut *host_ptr };
    let Some(exact_target) = window_task_target_for_runtime_owner(scope, host, target.owner) else {
        return Some(navigation_rejected_dom_exception_result(
            scope,
            "Navigation was canceled",
            "AbortError",
        ));
    };
    let _ = super::joint_history::position(scope, host, target.owner);
    if let Some((target_url, seed)) = history_entry_seed_for_traversal(
        scope,
        target.owner,
        target.current_index,
        target.target_index,
    ) {
        if runtime_window_is_global(scope, target.owner) {
            host.record_pending_location_navigation(target_url, Some(seed.clone()));
            return Some(navigation_pending_result(scope));
        }
        let target_key = traversal_target_entry(scope, &target)
            .and_then(|entry| navigation_entry_key_value(scope, entry));
        let receiver_context = target
            .history
            .get_creation_context(scope)
            .unwrap_or_else(|| scope.get_current_context());
        let receiver_scope = &mut v8::ContextScope::new(scope, receiver_context);
        let (result, producer) = host.queue_cross_document_history_traversal_with_result(
            receiver_scope,
            exact_target,
            target.target_index,
            target_key,
            target_url.as_str(),
            seed,
            info,
        )?;
        route_history_traversal_task(receiver_scope, host, producer);
        return Some(result);
    }
    let target_entry = traversal_target_entry(scope, &target);
    if !traversal_target_entry_still_available(scope, &target, target_entry) {
        return Some(navigation_rejected_dom_exception_result(
            scope,
            "Navigation was canceled",
            "AbortError",
        ));
    }
    let target_key = target_entry.and_then(|entry| navigation_entry_key_value(scope, entry));
    let receiver_context = target
        .history
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let receiver_scope = &mut v8::ContextScope::new(scope, receiver_context);
    let (result, producer) = host.queue_history_traversal_with_result(
        receiver_scope,
        exact_target,
        target.target_index,
        target_key,
        info,
    )?;
    if let Some(producer) = producer {
        route_history_traversal_task(receiver_scope, host, producer);
    }
    Some(result)
}

pub(super) fn queue_history_traversal_by_delta<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    delta: i64,
) {
    let owner = runtime_window_owner(scope, history);
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let Some(target) = window_task_target_for_runtime_owner(scope, host, owner) else {
        return;
    };
    // Isolated worlds have their own wrappers, not their own session history.
    let source_owner = match target.dispatch_scope() {
        crate::native_bridge::OwnerDispatchScope::Top => host
            .page_default_context(scope)
            .map(|context| context.global(scope)),
        _ => history_traversal_target_window(scope, host, target),
    };
    let Some(source_history) =
        source_owner.and_then(|owner| window_history_for_holder(scope, owner))
    else {
        return;
    };
    let source_index = history_index(scope, source_history);
    let source_entry_key = history_entries(scope, source_history)
        .and_then(|entries| entries.get_index(scope, source_index))
        .and_then(|entry| v8::Local::<v8::Object>::try_from(entry).ok())
        .and_then(|entry| navigation_entry_key_value(scope, entry));
    let joint = super::joint_history::position(scope, host, owner);
    let receiver_context = history
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let receiver_scope = &mut v8::ContextScope::new(scope, receiver_context);
    // Preserve the traversal queue's position separately from the script-visible
    // index, which synchronous navigations may update before this task runs.
    if let Some(producer) = host.queue_history_traversal_by_delta(
        receiver_scope,
        target,
        delta,
        source_index,
        source_entry_key,
        joint,
    ) {
        route_history_traversal_task(receiver_scope, host, producer);
    }
}

fn apply_history_traversal_by_delta(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    exact_target: WindowTaskTarget,
    delta: i64,
    current_index: u32,
) {
    let context = match exact_target.dispatch_scope() {
        crate::native_bridge::OwnerDispatchScope::Top => host.page_default_context(scope),
        _ => history_traversal_target_window(scope, host, exact_target)
            .and_then(|window| window.get_creation_context(scope)),
    };
    let Some(context) = context else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(owner) = history_traversal_target_window(scope, host, exact_target) else {
        return;
    };
    let Some(history) = window_history_for_holder(scope, owner) else {
        return;
    };
    let next_index = i64::from(current_index) + delta;
    let Some(entries) = history_entries(scope, history) else {
        return;
    };
    if next_index < 0 || next_index >= i64::from(entries.length()) {
        if runtime_window_is_global(scope, owner) {
            host.record_pending_top_level_history_traversal(delta);
        }
        return;
    }
    let target = TraversalTarget {
        owner,
        history,
        current_index,
        target_index: next_index as u32,
    };
    if let Some((target_url, seed)) = history_entry_seed_for_traversal(
        scope,
        target.owner,
        target.current_index,
        target.target_index,
    ) {
        if runtime_window_is_global(scope, target.owner) {
            host.record_pending_top_level_history_traversal(delta);
            return;
        }
        let target_key = traversal_target_entry(scope, &target)
            .and_then(|entry| navigation_entry_key_value(scope, entry));
        apply_pending_cross_document_traversal(
            scope,
            host,
            PendingCrossDocumentTraversal {
                target: exact_target,
                target_index: target.target_index,
                target_key,
                target_url: target_url.to_string(),
                seed,
                info: None,
                results: Vec::new(),
            },
        );
        return;
    }
    let target_key = traversal_target_entry(scope, &target)
        .and_then(|entry| navigation_entry_key_value(scope, entry));
    super::history_runtime::apply_pending_history_traversal(
        scope,
        host,
        PendingHistoryTraversal {
            target: exact_target,
            target_index: target.target_index,
            target_key,
            info: None,
            results: Vec::new(),
        },
    );
}

fn traversal_target_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: &TraversalTarget<'s>,
) -> Option<v8::Local<'s, v8::Object>> {
    history_entries(scope, target.history)?
        .get_index(scope, target.target_index)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn traversal_target_entry_still_available<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: &TraversalTarget<'s>,
    expected_entry: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    let Some(expected_entry) = expected_entry else {
        return true;
    };
    history_entries(scope, target.history)
        .and_then(|entries| entries.get_index(scope, target.target_index))
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .is_some_and(|entry| entry.strict_equals(expected_entry.into()))
}

fn traversal_target_key_still_available<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    target_index: u32,
    expected_key: Option<&str>,
) -> bool {
    let Some(expected_key) = expected_key else {
        return true;
    };
    history_entries(scope, history)
        .and_then(|entries| entries.get_index(scope, target_index))
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .and_then(|entry| navigation_entry_key_value(scope, entry))
        .is_some_and(|key| key == expected_key)
}

fn dispatch_traverse_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: &TraversalTarget<'s>,
) -> bool {
    let Some(navigation) = window_navigation_for_holder(scope, target.owner) else {
        return true;
    };
    dispatch_navigation_traverse_event(scope, navigation, target.history, target.target_index)
}

pub(in crate::context_bootstrap) fn apply_pending_cross_document_traversal<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    host: &mut JsContextHost,
    traversal: PendingCrossDocumentTraversal,
) -> bool {
    let Some(owner) = history_traversal_target_window(scope, host, traversal.target) else {
        reject_cross_document_traversal(scope, &traversal);
        return false;
    };
    let Some(history) = window_history_for_holder(scope, owner) else {
        reject_cross_document_traversal(scope, &traversal);
        return false;
    };
    if !traversal_target_key_still_available(
        scope,
        history,
        traversal.target_index,
        traversal.target_key.as_deref(),
    ) {
        reject_cross_document_traversal(scope, &traversal);
        return false;
    }
    let dispatch_scope = traversal.target.dispatch_scope();
    let retiring_document =
        host.current_window_document_task_target_for_dispatch_scope(dispatch_scope);
    let is_current = |scope: &mut v8::PinScope<'s, 'i>, host: &JsContextHost| {
        retiring_document.is_some()
            && host.current_window_document_task_target_for_dispatch_scope(dispatch_scope)
                == retiring_document
            && window_task_target_for_runtime_owner(scope, host, owner) == Some(traversal.target)
            && navigation_document_is_active(scope, owner)
    };
    if let crate::native_bridge::OwnerDispatchScope::Child(child_handle) = dispatch_scope {
        host.dispatch_child_document_tree_beforeunload_for_traversal(scope, child_handle);
    } else {
        dispatch_beforeunload_for_runtime_owner(scope, owner);
    }
    if !is_current(scope, host) {
        reject_cross_document_traversal(scope, &traversal);
        return false;
    }
    let info = traversal
        .info
        .as_ref()
        .map(|info| v8::Local::new(scope, info));
    let navigation = window_navigation_for_holder(scope, owner);
    if let Some(navigation) = navigation {
        cancel_active_cross_document_navigation(scope, navigation, None);
    }
    if !is_current(scope, host) {
        reject_cross_document_traversal(scope, &traversal);
        return false;
    }
    let outcome = navigation.map_or_else(NavigationDispatchOutcome::proceed, |navigation| {
        dispatch_navigation_traverse_event_with_outcome(
            scope,
            navigation,
            history,
            traversal.target_index,
            info,
        )
    });
    if let Some(error) = outcome.abort_error {
        // stop() aborts the API method and signal even though a cross-document
        // history traversal itself cannot be canceled by script.
        reject_pending_navigation_results(scope, &traversal.results, error);
    }
    // close() queues retirement of this exact Window. Keep its dispatched
    // event and API results alive until the close task aborts them together.
    let closing_popup = matches!(dispatch_scope,
        crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id)
            if host.lightweight_popup_is_closing(popup_id))
        && host.current_window_document_task_target_for_dispatch_scope(dispatch_scope)
            == retiring_document;
    if outcome.proceed
        && outcome.abort_error.is_none()
        && (is_current(scope, host) || closing_popup)
        && let Some(navigation) = navigation
    {
        track_cross_document_traversal_navigation(
            scope,
            navigation,
            outcome.signal,
            &traversal.target_url,
            &traversal.results,
        );
    }
    if !outcome.proceed || !is_current(scope, host) {
        if !closing_popup {
            reject_cross_document_traversal(scope, &traversal);
        }
        return false;
    }
    let queued = match dispatch_scope {
        crate::native_bridge::OwnerDispatchScope::Child(child_handle) => {
            queue_child_cross_document_traversal(
                host,
                child_handle,
                &traversal.target_url,
                traversal.seed,
            )
        }
        crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id) => host
            .queue_lightweight_popup_cross_document_traversal(
                scope,
                popup_id,
                &traversal.target_url,
                traversal.seed,
            ),
        crate::native_bridge::OwnerDispatchScope::Top => {
            unreachable!("top-level traversals are handed off to the browser")
        }
    };
    if !queued && let Some(navigation) = navigation {
        cancel_active_cross_document_navigation(scope, navigation, None);
    }
    queued
}

pub(crate) fn apply_authorized_history_traversal_task(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    action: PendingHistoryTraversalAction,
) {
    match action {
        PendingHistoryTraversalAction::ByDelta {
            target,
            delta,
            position,
        } => {
            let current_index = position.borrow().index;
            let joint = position.borrow().joint;
            let previous = host.replace_active_history_delta_position(Some((target, position)));
            if let Some(joint) = joint {
                super::joint_history::apply_delta(scope, host, target, joint, delta);
            } else {
                apply_history_traversal_by_delta(scope, host, target, delta, current_index);
            }
            host.replace_active_history_delta_position(previous);
        }
        PendingHistoryTraversalAction::SameDocument(traversal) => {
            super::joint_history::apply_entry(scope, host, traversal);
        }
        PendingHistoryTraversalAction::CrossDocument(traversal) => {
            super::joint_history::apply_entry(
                scope,
                host,
                PendingHistoryTraversal {
                    target: traversal.target,
                    target_index: traversal.target_index,
                    target_key: traversal.target_key,
                    info: traversal.info,
                    results: traversal.results,
                },
            );
        }
    }
}

fn reject_cross_document_traversal(
    scope: &mut v8::PinScope<'_, '_>,
    traversal: &PendingCrossDocumentTraversal,
) {
    reject_canceled_history_traversal_results(scope, &traversal.results);
}

pub(crate) fn reject_canceled_history_traversal_results(
    scope: &mut v8::PinScope<'_, '_>,
    results: &[PendingNavigationResult],
) {
    if results.is_empty() {
        return;
    }
    let error = navigation_dom_exception(scope, "Navigation was canceled", "AbortError");
    reject_pending_navigation_results(scope, results, error);
}

fn queue_child_cross_document_traversal(
    host: &mut JsContextHost,
    child_handle: DomHandle,
    target_url: &str,
    seed: NavigationHistoryEntrySeed,
) -> bool {
    host.queue_deferred_child_browsing_context_navigation_from_entry_seed(
        child_handle,
        target_url,
        seed,
        false,
        None,
    )
}
