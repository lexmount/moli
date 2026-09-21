use super::history_runtime::{
    history_traversal_target_window, reject_pending_navigation_results, route_history_traversal_task,
};
use super::navigation_entry::{history_entries};
use super::navigation_events::{
    NavigationDispatchOutcome, dispatch_beforeunload_for_runtime_owner,
    dispatch_navigation_traverse_event_with_outcome,
};
use super::navigation_result::{
    cancel_active_cross_document_navigation, navigation_dom_exception,
    navigation_pending_result,
    navigation_rejected_dom_exception_result, track_cross_document_traversal_navigation,
};
use super::navigation_seed::history_entry_seed_for_traversal;
use super::navigation_window::{navigation_document_is_active, window_navigation_for_holder};
use super::navigation_window::{
    runtime_window_is_global,
    window_history_for_holder, window_task_target_for_runtime_owner,
};
use super::*;
use crate::native_bridge::{
    PendingCrossDocumentTraversal,
    PendingHistoryTraversalAction, PendingNavigationResult,
};
use moli_history::HistoryEntryRef;

pub(super) struct TraversalTarget<'s> {
    pub(super) owner: v8::Local<'s, v8::Object>,
    pub(super) history: v8::Local<'s, v8::Object>,
    pub(super) current_index: u32,
    pub(super) target_index: u32,
    pub(super) joint_step: Option<moli_session_history::SessionHistoryStepId>,
}

pub(super) fn queue_navigation_traversal_with_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    mut target: TraversalTarget<'s>,
    info: Option<v8::Local<'s, v8::Value>>,
) -> Option<v8::Local<'s, v8::Object>> {
    target.joint_step = target.joint_step.or_else(|| {
        traversal_target_entry(scope, &target)
            .and_then(|entry| super::session_history::step_for_entry(scope, target.owner, &entry))
    });
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return Some(navigation_rejected_dom_exception_result(scope, "Navigation was canceled", "AbortError"));
    };
    let host = unsafe { &mut *host_ptr };
    let Some(exact_target) = window_task_target_for_runtime_owner(scope, host, target.owner) else {
        return Some(navigation_rejected_dom_exception_result(
            scope,
            "Navigation was canceled",
            "AbortError",
        ));
    };
    if let Some((target_url, mut seed)) = history_entry_seed_for_traversal(
        scope,
        target.owner,
        target.current_index,
        target.target_index,
    ) {
        seed.session_history.target_step = target.joint_step;
        super::session_history::capture_for_navigation(scope, target.owner, &mut seed);
        if runtime_window_is_global(scope, target.owner) {
            host.record_pending_location_navigation(target_url, Some(seed.clone()));
            return Some(navigation_pending_result(scope));
        }
        let target_key = traversal_target_entry(scope, &target)
            .map(|entry| entry.borrow().key.as_str().to_owned());
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
            seed,
            info,
        )?;
        route_history_traversal_task(receiver_scope, host, producer);
        return Some(result);
    }
    let target_entry = traversal_target_entry(scope, &target);
    if !traversal_target_entry_still_available(scope, &target, target_entry.as_ref()) {
        return Some(navigation_rejected_dom_exception_result(
            scope,
            "Navigation was canceled",
            "AbortError",
        ));
    }
    let target_key = target_entry.map(|entry| entry.borrow().key.as_str().to_owned());
    let receiver_context = target
        .history
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let receiver_scope = &mut v8::ContextScope::new(scope, receiver_context);
    let (result, producer) = host.queue_history_traversal_with_result(
        receiver_scope,
        exact_target,
        target.target_index,
        target.joint_step,
        target_key,
        info,
    )?;
    if let Some(producer) = producer {
        route_history_traversal_task(receiver_scope, host, producer);
    }
    Some(result)
}

pub(super) fn queue_history_traversal_without_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    mut target: TraversalTarget<'s>,
) {
    target.joint_step = target.joint_step.or_else(|| {
        traversal_target_entry(scope, &target)
            .and_then(|entry| super::session_history::step_for_entry(scope, target.owner, &entry))
    });
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let Some(exact_target) = window_task_target_for_runtime_owner(scope, host, target.owner) else {
        return;
    };
    if let Some((_target_url, mut seed)) = history_entry_seed_for_traversal(
        scope,
        target.owner,
        target.current_index,
        target.target_index,
    ) {
        seed.session_history.target_step = target.joint_step;
        super::session_history::capture_for_navigation(scope, target.owner, &mut seed);
        if runtime_window_is_global(scope, target.owner) {
            let binding = super::session_history::binding(scope, host, target.owner);
            let Some(delta) = target
                .joint_step
                .and_then(|step| host.session_histories.get_mut(binding.popup).delta_to(step))
            else {
                return;
            };
            host.record_pending_top_level_history_traversal(delta);
            return;
        }
        let target_key = traversal_target_entry(scope, &target)
            .map(|entry| entry.borrow().key.as_str().to_owned());
        let receiver_context = target.history.get_creation_context(scope)
            .unwrap_or_else(|| scope.get_current_context());
        let receiver_scope = &mut v8::ContextScope::new(scope, receiver_context);
        if let Some(producer) = host.queue_cross_document_history_traversal(
            receiver_scope, exact_target, target.target_index, target_key, seed,
        ) {
            route_history_traversal_task(receiver_scope, host, producer);
        }
        return;
    }
    let target_entry = traversal_target_entry(scope, &target);
    if !traversal_target_entry_still_available(scope, &target, target_entry.as_ref()) {
        return;
    }
    let receiver_context = target
        .history
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let receiver_scope = &mut v8::ContextScope::new(scope, receiver_context);
    let target_key = target_entry.map(|entry| entry.borrow().key.as_str().to_owned());
    if let Some(producer) = host.queue_history_traversal(
        receiver_scope,
        exact_target,
        target.target_index,
        target.joint_step,
        target_key,
    ) {
        route_history_traversal_task(receiver_scope, host, producer);
    }
}

fn traversal_target_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: &TraversalTarget<'s>,
) -> Option<HistoryEntryRef> {
    history_entries(scope, target.history)?
        .get(target.target_index as usize)
        .cloned()
}

fn traversal_target_entry_still_available<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: &TraversalTarget<'s>,
    expected_entry: Option<&HistoryEntryRef>,
) -> bool {
    let Some(expected_entry) = expected_entry else {
        return true;
    };
    traversal_target_entry(scope, target)
        .is_some_and(|entry| std::rc::Rc::ptr_eq(&entry, expected_entry))
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
    history_entries(scope, history).is_some_and(|entries| {
        entries.get(target_index as usize)
            .is_some_and(|entry| entry.borrow().key.as_str() == expected_key)
    })
}

pub(in crate::context_bootstrap) fn apply_pending_cross_document_traversal<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    host: &mut JsContextHost,
    traversal: PendingCrossDocumentTraversal,
) {
    if matches!(traversal.target.dispatch_scope(), crate::native_bridge::OwnerDispatchScope::Child(_)) {
        super::history_runtime::apply_pending_history_traversal(
            scope, host,
            crate::native_bridge::PendingHistoryTraversal {
                joint_step: traversal.seed.session_history.target_step,
                target: traversal.target,
                target_index: traversal.target_index,
                target_key: traversal.target_key,
                info: traversal.info,
                results: traversal.results,
            },
        );
        return;
    }
    let Some(target_url) = traversal.seed.entries.iter()
        .find(|entry| entry.history_index == traversal.seed.current_index)
        .map(|entry| entry.url.clone())
    else {
        reject_cross_document_traversal(scope, &traversal);
        return;
    };
    let Some(owner) = history_traversal_target_window(scope, host, traversal.target) else {
        reject_cross_document_traversal(scope, &traversal);
        return;
    };
    let Some(history) = window_history_for_holder(scope, owner) else {
        reject_cross_document_traversal(scope, &traversal);
        return;
    };
    if !traversal_target_key_still_available(
        scope,
        history,
        traversal.target_index,
        traversal.target_key.as_deref(),
    ) {
        reject_cross_document_traversal(scope, &traversal);
        return;
    }
    let info = traversal
        .info
        .as_ref()
        .map(|info| v8::Local::new(scope, info));
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
        return;
    }
    let navigation = window_navigation_for_holder(scope, owner);
    if let Some(navigation) = navigation {
        cancel_active_cross_document_navigation(scope, navigation, None);
    }
    if !is_current(scope, host) {
        reject_cross_document_traversal(scope, &traversal);
        return;
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
            &target_url,
            &traversal.results,
        );
    }
    // Aborting the API event during dispatch sets its canceled flag, but
    // cross-document traversals cannot be canceled by script. Only continue
    // while the original Window and Document are still active.
    if !is_current(scope, host) {
        if !closing_popup {
            reject_cross_document_traversal(scope, &traversal);
        }
        return;
    }
    match dispatch_scope {
        crate::native_bridge::OwnerDispatchScope::Child(_) => unreachable!("child traversal uses the coordinator"),
        crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id) => {
            if !host.queue_lightweight_popup_cross_document_traversal(
                scope,
                popup_id,
                &target_url,
                traversal.seed,
            ) && let Some(navigation) = navigation
            {
                cancel_active_cross_document_navigation(scope, navigation, None);
            }
        }
        crate::native_bridge::OwnerDispatchScope::Top => {
            unreachable!("top-level traversals are handed off to the browser")
        }
    }
}

pub(crate) fn apply_authorized_history_traversal_task(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    action: PendingHistoryTraversalAction,
) {
    match action {
        PendingHistoryTraversalAction::SameDocument(traversal) => {
            super::history_runtime::apply_pending_history_traversal(scope, host, traversal);
        }
        PendingHistoryTraversalAction::CrossDocument(traversal) => {
            apply_pending_cross_document_traversal(scope, host, *traversal);
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
