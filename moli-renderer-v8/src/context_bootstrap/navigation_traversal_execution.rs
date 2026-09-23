use super::history_runtime::{
    apply_history_entry, reject_pending_navigation_results, route_history_traversal_task,
};
use super::navigation_entry::{history_entries, history_index, navigation_entry_key_value};
use super::navigation_events::{
    dispatch_beforeunload_for_runtime_owner, dispatch_navigation_traverse_event,
    dispatch_navigation_traverse_event_with_outcome, dispatch_pagehide_for_runtime_owner,
    dispatch_unload_for_runtime_owner, mark_navigation_outcome_default_prevented,
};
use super::navigation_result::{
    navigation_dom_exception, navigation_immediate_current_entry_result, navigation_pending_result,
    navigation_rejected_dom_exception_result,
};
use super::navigation_seed::history_entry_seed_for_traversal;
use super::navigation_window::{
    child_browsing_context_handle_for_runtime_owner, runtime_window_is_global,
    window_history_for_holder, window_task_target_for_runtime_owner,
};
use super::navigation_window::{navigation_document_is_active, window_navigation_for_holder};
use super::*;
use crate::document_runtime::DomHandle;
use crate::native_bridge::{
    NavigationHistoryEntrySeed, PendingChildCrossDocumentTraversal, PendingHistoryTraversalAction,
};

pub(super) struct TraversalTarget<'s> {
    pub(super) owner: v8::Local<'s, v8::Object>,
    pub(super) history: v8::Local<'s, v8::Object>,
    pub(super) current_index: u32,
    pub(super) target_index: u32,
    pub(super) joint_step: Option<moli_page_types::SessionHistoryStepId>,
}

pub(super) fn queue_navigation_traversal_with_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    mut target: TraversalTarget<'s>,
    info: Option<v8::Local<'s, v8::Value>>,
) -> Option<v8::Local<'s, v8::Object>> {
    target.joint_step = target.joint_step.or_else(|| {
        traversal_target_entry(scope, &target)
            .and_then(|entry| super::session_history::step_for_entry(scope, target.owner, entry))
    });
    let child_handle = child_browsing_context_handle_for_traversal(scope, target.owner);
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        if !dispatch_traverse_event(scope, &target) {
            return Some(navigation_rejected_dom_exception_result(
                scope,
                "Navigation was canceled",
                "AbortError",
            ));
        }
        apply_history_entry(
            scope,
            target.history,
            target.target_index,
            target.joint_step,
            true,
            None,
        );
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
        if let Some(popup_id) =
            crate::native_bridge::lightweight_popup_id_from_window(scope, target.owner)
        {
            host.queue_lightweight_popup_cross_document_traversal(
                scope,
                popup_id,
                target_url.as_str(),
                seed,
            );
            return Some(navigation_pending_result(scope));
        }
        let Some(child_handle) = child_handle else {
            return Some(navigation_pending_result(scope));
        };
        let target_key = traversal_target_entry(scope, &target)
            .and_then(|entry| navigation_entry_key_value(scope, entry));
        let receiver_context = target
            .history
            .get_creation_context(scope)
            .unwrap_or_else(|| scope.get_current_context());
        let receiver_scope = &mut v8::ContextScope::new(scope, receiver_context);
        let (result, producer) = host.queue_child_cross_document_traversal_with_result(
            receiver_scope,
            exact_target,
            child_handle,
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
            .and_then(|entry| super::session_history::step_for_entry(scope, target.owner, entry))
    });
    let child_handle = child_browsing_context_handle_for_traversal(scope, target.owner);
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        if !dispatch_traverse_event(scope, &target) {
            return;
        }
        apply_history_entry(
            scope,
            target.history,
            target.target_index,
            target.joint_step,
            true,
            None,
        );
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let Some(exact_target) = window_task_target_for_runtime_owner(scope, host, target.owner) else {
        return;
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
        if let Some(popup_id) =
            crate::native_bridge::lightweight_popup_id_from_window(scope, target.owner)
        {
            host.queue_lightweight_popup_cross_document_traversal(
                scope,
                popup_id,
                target_url.as_str(),
                seed,
            );
            return;
        }
        if let Some(child_handle) = child_handle {
            let target_key = traversal_target_entry(scope, &target)
                .and_then(|entry| navigation_entry_key_value(scope, entry));
            let receiver_context = target
                .history
                .get_creation_context(scope)
                .unwrap_or_else(|| scope.get_current_context());
            let receiver_scope = &mut v8::ContextScope::new(scope, receiver_context);
            if let Some(producer) = host.queue_child_cross_document_traversal(
                receiver_scope,
                exact_target,
                child_handle,
                target.target_index,
                target_key,
                target_url.as_str(),
                seed,
            ) {
                route_history_traversal_task(receiver_scope, host, producer);
            }
        }
        return;
    }
    let target_entry = traversal_target_entry(scope, &target);
    if !traversal_target_entry_still_available(scope, &target, target_entry) {
        return;
    }
    let receiver_context = target
        .history
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let receiver_scope = &mut v8::ContextScope::new(scope, receiver_context);
    let target_key =
        target_entry.and_then(|entry| navigation_entry_key_value(receiver_scope, entry));
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

fn dispatch_child_cross_document_traverse_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: &TraversalTarget<'s>,
    info: Option<v8::Local<'s, v8::Value>>,
) -> bool {
    dispatch_beforeunload_for_runtime_owner(scope, target.owner);
    let proceed = window_navigation_for_holder(scope, target.owner).is_none_or(|navigation| {
        let outcome = dispatch_navigation_traverse_event_with_outcome(
            scope,
            navigation,
            target.history,
            target.target_index,
            info,
        );
        if !navigation_document_is_active(scope, target.owner) {
            mark_navigation_outcome_default_prevented(scope, &outcome);
            return false;
        }
        outcome.proceed
    });
    if proceed {
        dispatch_pagehide_for_runtime_owner(scope, target.owner);
        dispatch_unload_for_runtime_owner(scope, target.owner);
    }
    proceed
}

pub(in crate::context_bootstrap) fn apply_pending_child_cross_document_traversal(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    traversal: PendingChildCrossDocumentTraversal,
) {
    let Some(owner) = host.child_browsing_context_window_wrapper(scope, traversal.child_handle)
    else {
        reject_child_cross_document_traversal(scope, &traversal);
        return;
    };
    let Some(history) = window_history_for_holder(scope, owner) else {
        reject_child_cross_document_traversal(scope, &traversal);
        return;
    };
    let target = TraversalTarget {
        owner,
        history,
        current_index: history_index(scope, history),
        target_index: traversal.target_index,
        joint_step: traversal.seed.session_history.target_step,
    };
    if !traversal_target_key_still_available(
        scope,
        history,
        traversal.target_index,
        traversal.target_key.as_deref(),
    ) {
        reject_child_cross_document_traversal(scope, &traversal);
        return;
    }
    let info = traversal
        .info
        .as_ref()
        .map(|info| v8::Local::new(scope, info));
    if !dispatch_child_cross_document_traverse_event(scope, &target, info) {
        reject_child_cross_document_traversal(scope, &traversal);
        return;
    }
    let _ = host.mark_current_child_document_unload_dispatched_after_navigation_traversal(
        traversal.child_handle,
    );
    queue_child_cross_document_traversal(
        host,
        traversal.child_handle,
        &traversal.target_url,
        traversal.seed,
    );
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
        PendingHistoryTraversalAction::ChildCrossDocument(traversal) => {
            apply_pending_child_cross_document_traversal(scope, host, *traversal);
        }
    }
}

fn reject_child_cross_document_traversal(
    scope: &mut v8::PinScope<'_, '_>,
    traversal: &PendingChildCrossDocumentTraversal,
) {
    if traversal.results.is_empty() {
        return;
    }
    let error = navigation_dom_exception(scope, "Navigation was canceled", "AbortError");
    reject_pending_navigation_results(scope, &traversal.results, error);
}

fn child_browsing_context_handle_for_traversal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    if runtime_window_is_global(scope, owner) {
        None
    } else {
        child_browsing_context_handle_for_runtime_owner(scope, owner)
    }
}

fn queue_child_cross_document_traversal(
    host: &mut JsContextHost,
    child_handle: DomHandle,
    target_url: &str,
    seed: NavigationHistoryEntrySeed,
) {
    host.queue_deferred_child_browsing_context_navigation_from_entry_seed(
        child_handle,
        target_url,
        seed,
    );
}
