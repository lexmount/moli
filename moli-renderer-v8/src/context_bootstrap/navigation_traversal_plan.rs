use super::history_runtime::pending_history_traversal_target_index;
use super::navigation_entry::{
    history_entries, history_index, navigation_current_entry, navigation_current_entry_index,
};
use super::navigation_projection::{
    visible_navigation_entries_len, visible_navigation_index_for_entry,
};
use super::navigation_traversal_execution::TraversalTarget;
use super::navigation_window::{
    navigation_document_has_opaque_origin, navigation_document_is_active, runtime_window_owner,
    window_history_for_holder,
};

pub(super) enum NavigationTraversalPlan<'s> {
    RejectInvalidState(&'static str),
    ResolveCurrentEntry(v8::Local<'s, v8::Object>),
    Traverse(TraversalTarget<'s>),
}

pub(super) fn navigation_delta_traversal_plan<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    delta: i64,
) -> Option<NavigationTraversalPlan<'s>> {
    let owner = runtime_window_owner(scope, navigation);
    if !navigation_document_is_active(scope, owner) {
        return Some(NavigationTraversalPlan::RejectInvalidState(
            "Cannot traverse a non-fully-active document",
        ));
    }
    if navigation_document_has_opaque_origin(scope, owner) {
        return Some(NavigationTraversalPlan::RejectInvalidState(
            "Cannot traverse an opaque origin document",
        ));
    }
    let history = window_history_for_holder(scope, owner)?;
    let current_index = pending_history_traversal_target_index(scope, history)
        .unwrap_or_else(|| history_index(scope, history)) as i64;
    let entries = history_entries(scope, history)?;
    let current_entry = navigation_current_entry(scope, owner);
    let current_navigation_index = current_entry
        .and_then(|entry| visible_navigation_index_for_entry(scope, entries, Some(entry), entry))
        .or_else(|| navigation_current_entry_index(scope, owner))
        .unwrap_or(0) as i64;
    let visible_len = visible_navigation_entries_len(scope, entries, current_entry) as i64;
    if delta < 0 && current_navigation_index <= 0 {
        return Some(NavigationTraversalPlan::RejectInvalidState(
            "Cannot go back",
        ));
    }
    if delta > 0 && current_navigation_index + delta >= visible_len {
        return Some(NavigationTraversalPlan::RejectInvalidState(
            "Cannot go forward",
        ));
    }
    let next_index = current_index + delta;
    if next_index < 0 || next_index >= entries.length() as i64 {
        let message = if delta < 0 {
            "Cannot go back"
        } else {
            "Cannot go forward"
        };
        return Some(NavigationTraversalPlan::RejectInvalidState(message));
    }
    Some(NavigationTraversalPlan::Traverse(TraversalTarget {
        owner,
        history,
        current_index: current_index as u32,
        target_index: next_index as u32,
    }))
}

pub(super) fn navigation_index_traversal_plan<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    target_index: u32,
) -> Option<NavigationTraversalPlan<'s>> {
    let owner = runtime_window_owner(scope, navigation);
    if !navigation_document_is_active(scope, owner) {
        return Some(NavigationTraversalPlan::RejectInvalidState(
            "Cannot traverse a non-fully-active document",
        ));
    }
    if navigation_document_has_opaque_origin(scope, owner) {
        return Some(NavigationTraversalPlan::RejectInvalidState(
            "Cannot traverse an opaque origin document",
        ));
    }
    let history = window_history_for_holder(scope, owner)?;
    let entries = history_entries(scope, history)?;
    if target_index >= entries.length() {
        return Some(NavigationTraversalPlan::RejectInvalidState("Invalid key"));
    }
    let pending_target_index = pending_history_traversal_target_index(scope, history);
    if pending_target_index == Some(target_index) {
        return Some(NavigationTraversalPlan::Traverse(TraversalTarget {
            owner,
            history,
            current_index: history_index(scope, history),
            target_index,
        }));
    }
    let current_index = pending_target_index.unwrap_or_else(|| history_index(scope, history));
    if current_index == target_index {
        return Some(NavigationTraversalPlan::ResolveCurrentEntry(owner));
    }
    Some(NavigationTraversalPlan::Traverse(TraversalTarget {
        owner,
        history,
        current_index,
        target_index,
    }))
}

pub(super) fn history_delta_traversal_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    delta: i64,
) -> Option<TraversalTarget<'s>> {
    let current_index = i64::from(history_index(scope, history));
    let entries = history_entries(scope, history)?;
    let next_index = current_index + delta;
    if next_index < 0 || next_index >= entries.length() as i64 {
        return None;
    }
    let owner = runtime_window_owner(scope, history);
    Some(TraversalTarget {
        history,
        owner,
        current_index: current_index as u32,
        target_index: next_index as u32,
    })
}
