use super::location_navigation::NavigationNavigateHistoryKind;
use super::location_runtime::location_href_slot;
use super::navigation_entry::{
    history_index, navigation_current_entry, navigation_current_entry_index,
};
use super::navigation_events::{
    dispatch_beforeunload_for_runtime_owner, dispatch_pagehide_for_runtime_owner,
    dispatch_unload_for_runtime_owner,
};
use super::navigation_result::{
    navigation_cross_document_pending_result, navigation_pending_result,
};
use super::navigation_serialize::{
    apply_current_document_referrer_policy_to_entry_snapshots, serialize_history_entries,
    serialize_navigation_entry_object,
};
use super::navigation_window::{
    child_browsing_context_handle_for_runtime_owner, navigation_document_is_initial_empty,
    runtime_window_is_global, window_location_for_holder,
};
use super::*;
use moli_page_types::{NavigationHistoryMutation, cross_document_navigation_seed};

pub(super) fn handle_navigation_navigate_cross_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    history: v8::Local<'s, v8::Object>,
    next_url: &url::Url,
    navigate_history_kind: NavigationNavigateHistoryKind,
    navigation_signal: Option<(v8::Local<'s, v8::Object>, Option<v8::Local<'s, v8::Object>>)>,
) -> v8::Local<'s, v8::Value> {
    let current_navigation_index = navigation_current_entry_index(scope, owner).unwrap_or(0);
    let current_index = history_index(scope, history);
    let mut entries = serialize_history_entries(scope, history);
    if let Some(current_entry) = navigation_current_entry(scope, owner) {
        let current_snapshot = serialize_navigation_entry_object(scope, current_entry, &entries);
        if let Some(existing) = entries
            .iter_mut()
            .find(|entry| entry.history_index == current_index)
        {
            *existing = current_snapshot;
        }
    }
    apply_current_document_referrer_policy_to_entry_snapshots(
        scope,
        owner,
        current_index,
        &mut entries,
    );
    let current_href = window_location_for_holder(scope, owner)
        .and_then(|location| location_href_slot(scope, location));
    let current_entry_is_about_blank = entries
        .iter()
        .find(|entry| entry.history_index == current_index)
        .is_some_and(|entry| entry.url == "about:blank");
    let replace_initial_empty_document = navigation_document_is_initial_empty(scope, owner);
    let mutation = match navigate_history_kind {
        NavigationNavigateHistoryKind::Push if replace_initial_empty_document => {
            NavigationHistoryMutation::Replace
        }
        NavigationNavigateHistoryKind::Push => NavigationHistoryMutation::Push,
        NavigationNavigateHistoryKind::Replace => NavigationHistoryMutation::Replace,
        NavigationNavigateHistoryKind::Default => {
            if replace_initial_empty_document
                || (current_href.as_deref() == Some("about:blank") && current_entry_is_about_blank)
            {
                NavigationHistoryMutation::Replace
            } else {
                NavigationHistoryMutation::Push
            }
        }
    };

    if runtime_window_is_global(scope, owner) {
        let entry_seed = cross_document_navigation_seed(
            entries,
            current_index,
            current_navigation_index,
            next_url,
            mutation,
        );
        dispatch_beforeunload_for_runtime_owner(scope, owner);
        dispatch_pagehide_for_runtime_owner(scope, owner);
        dispatch_unload_for_runtime_owner(scope, owner);
        let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
            return v8::undefined(scope).into();
        };
        unsafe { &mut *host_ptr }
            .record_pending_location_navigation(next_url.clone(), Some(entry_seed));
        return navigation_signal
            .map(|(navigation, signal)| {
                navigation_cross_document_pending_result(
                    scope,
                    navigation,
                    signal,
                    next_url.as_str(),
                )
            })
            .unwrap_or_else(|| navigation_pending_result(scope))
            .into();
    }

    let Some(child_handle) = child_browsing_context_handle_for_runtime_owner(scope, owner) else {
        return navigation_pending_result(scope).into();
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return navigation_pending_result(scope).into();
    };
    let entry_seed = cross_document_navigation_seed(
        entries,
        current_index,
        current_navigation_index,
        next_url,
        mutation,
    );
    dispatch_beforeunload_for_runtime_owner(scope, owner);
    let host = unsafe { &mut *host_ptr };
    host.queue_deferred_child_browsing_context_navigation_from_entry_seed(
        child_handle,
        next_url.as_str(),
        entry_seed,
    );
    host.sync_existing_child_browsing_context_window_state(scope, child_handle);
    navigation_signal
        .map(|(navigation, signal)| {
            navigation_cross_document_pending_result(scope, navigation, signal, next_url.as_str())
        })
        .unwrap_or_else(|| navigation_pending_result(scope))
        .into()
}
