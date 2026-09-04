use super::super::location_runtime::{
    is_same_document_fragment_navigation, location_href_slot, sync_location_object,
};
use super::super::navigation_entry::{
    history_entries, history_index, navigation_current_entry, navigation_current_entry_index,
    navigation_entries_share_document, navigation_entry_initial_index,
    navigation_entry_joint_top_index, navigation_entry_url_value,
    restore_current_navigation_entry_scroll_position, set_history_index, set_history_state,
    sync_navigation_current_entry_from_history_entry,
};
use super::super::navigation_events::{
    dispatch_navigation_currententrychange, dispatch_navigation_success, dispatch_popstate_event,
    navigation_has_active_scroll_event, queue_hash_change_for_runtime_owner,
};
use super::super::navigation_mutation::sync_local_document_front_from_window;
use super::super::navigation_projection::build_visible_navigation_entries_array;
use super::super::navigation_result::perform_navigation_scroll_if_needed;
use super::super::navigation_serialize::sync_child_navigation_entry_seed_from_owner;
use super::super::navigation_window::{
    navigation_document_has_opaque_origin, runtime_top_window_owner, runtime_window_is_global,
    runtime_window_owner, window_history_for_holder, window_location_for_holder,
    window_navigation_for_holder,
};
use super::super::*;
use super::results::{resolve_pending_navigation_committed, resolve_pending_navigation_finished};
use crate::native_bridge::PendingNavigationResult;
use crate::script_vm::perform_microtask_checkpoint_and_report_pending_promise_rejections;
use moli_page_types::SameDocumentHistoryUpdate;

pub(in crate::context_bootstrap) struct AppliedHistoryEntry<'s> {
    pub(in crate::context_bootstrap) owner: v8::Local<'s, v8::Object>,
    pub(in crate::context_bootstrap) state: v8::Local<'s, v8::Value>,
    pub(in crate::context_bootstrap) old_url: Option<String>,
    pub(in crate::context_bootstrap) url: String,
    pub(in crate::context_bootstrap) parsed_url: url::Url,
    pub(in crate::context_bootstrap) resolved_entry: v8::Local<'s, v8::Value>,
    previous_history_index: u32,
    history_index: u32,
    entry: v8::Local<'s, v8::Object>,
    previous_entry: Option<v8::Local<'s, v8::Object>>,
}

pub(in crate::context_bootstrap) fn apply_history_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    index: u32,
    dispatch_popstate: bool,
    pending_results: Option<&[PendingNavigationResult]>,
) {
    let Some(applied) = apply_history_entry_commit(scope, history, index) else {
        return;
    };
    if let Some(results) = pending_results {
        resolve_pending_navigation_committed(scope, results, applied.resolved_entry);
    }
    dispatch_history_entry_currententrychange(scope, &applied);
    if dispatch_popstate {
        perform_microtask_checkpoint_and_report_pending_promise_rejections(scope);
    }
    if !navigation_document_has_opaque_origin(scope, applied.owner)
        && let Some(navigation) = window_navigation_for_holder(scope, applied.owner)
    {
        let has_active_scroll_event = navigation_has_active_scroll_event(scope, navigation);
        if has_active_scroll_event
            || !restore_current_navigation_entry_scroll_position(scope, applied.owner)
        {
            perform_navigation_scroll_if_needed(scope, navigation, &applied.url, true);
        }
        dispatch_navigation_success(scope, navigation);
    }
    if let Some(results) = pending_results {
        resolve_pending_navigation_finished(scope, results, applied.resolved_entry);
    }
    if dispatch_popstate {
        perform_microtask_checkpoint_and_report_pending_promise_rejections(scope);
    }
    dispatch_history_entry_post_commit_events(scope, &applied, dispatch_popstate);
}

pub(in crate::context_bootstrap) fn apply_history_entry_commit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    index: u32,
) -> Option<AppliedHistoryEntry<'s>> {
    let owner = runtime_window_owner(scope, history);
    let previous_history_index = history_index(scope, history);
    let previous_entry = navigation_current_entry(scope, owner);
    let old_url = window_location_for_holder(scope, owner)
        .and_then(|location| location_href_slot(scope, location));
    let entries = history_entries(scope, history)?;
    let entry = entries
        .get_index(scope, index)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let state = super::super::navigation_entry_state::clone_history_entry_state(scope, entry)
        .unwrap_or_else(|| v8::null(scope).into());
    let url = navigation_entry_url_value(scope, entry).unwrap_or_else(|| "about:blank".to_owned());
    set_history_index(scope, history, index);
    set_history_state(scope, history, state);

    let location = window_location_for_holder(scope, owner)?;
    sync_location_object(scope, location, &url);
    let Ok(parsed_url) = url::Url::parse(&url) else {
        return None;
    };
    sync_navigation_current_entry_from_history_entry(scope, owner, entry);
    sync_top_history_after_child_traversal(scope, owner, entry, previous_entry);
    let resolved_entry = navigation_current_entry(scope, owner)
        .map(v8::Local::<v8::Value>::from)
        .unwrap_or_else(|| v8::undefined(scope).into());
    sync_child_navigation_entry_seed_from_owner(scope, owner);
    Some(AppliedHistoryEntry {
        owner,
        state,
        old_url,
        url,
        parsed_url,
        resolved_entry,
        previous_history_index,
        history_index: index,
        entry,
        previous_entry,
    })
}

fn sync_top_history_after_child_traversal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    child_entry: v8::Local<'s, v8::Object>,
    previous_child_entry: Option<v8::Local<'s, v8::Object>>,
) {
    if runtime_window_is_global(scope, owner) {
        return;
    }
    let Some(previous_child_index) =
        previous_child_entry.and_then(|entry| navigation_entry_initial_index(scope, entry))
    else {
        return;
    };
    let Some(target_index) = navigation_entry_joint_top_index(scope, child_entry)
        .or_else(|| navigation_entry_initial_index(scope, child_entry))
    else {
        return;
    };
    let top_owner = runtime_top_window_owner(scope, owner);
    if top_owner.strict_equals(owner.into()) {
        return;
    }
    if navigation_current_entry_index(scope, top_owner)
        .is_none_or(|top_index| top_index <= previous_child_index)
    {
        return;
    }
    let Some(top_history) = window_history_for_holder(scope, top_owner) else {
        return;
    };
    let Some(top_entries) = history_entries(scope, top_history) else {
        return;
    };
    let top_current_entry = navigation_current_entry(scope, top_owner);
    let visible_entries =
        build_visible_navigation_entries_array(scope, top_entries, top_current_entry);
    let Some(top_entry) = visible_entries
        .get_index(scope, target_index)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let Some(raw_index) = raw_index_for_entry(scope, top_entries, top_entry) else {
        return;
    };

    set_history_index(scope, top_history, raw_index);
    let state = super::super::navigation_entry_state::clone_history_entry_state(scope, top_entry)
        .unwrap_or_else(|| v8::null(scope).into());
    set_history_state(scope, top_history, state);
    if let Some(url) = navigation_entry_url_value(scope, top_entry)
        && let Some(location) = window_location_for_holder(scope, top_owner)
    {
        sync_location_object(scope, location, &url);
    }
    sync_navigation_current_entry_from_history_entry(scope, top_owner, top_entry);
}

fn raw_index_for_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: v8::Local<'s, v8::Array>,
    target: v8::Local<'s, v8::Object>,
) -> Option<u32> {
    (0..entries.length()).find(|index| {
        entries
            .get_index(scope, *index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .is_some_and(|entry| entry.strict_equals(target.into()))
    })
}

pub(in crate::context_bootstrap) fn dispatch_history_entry_currententrychange<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    applied: &AppliedHistoryEntry<'s>,
) {
    if !navigation_document_has_opaque_origin(scope, applied.owner)
        && let Some(navigation) = window_navigation_for_holder(scope, applied.owner)
    {
        dispatch_navigation_currententrychange(
            scope,
            navigation,
            applied.previous_entry,
            Some("traverse"),
        );
    }
}

pub(in crate::context_bootstrap) fn dispatch_history_entry_post_commit_events<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    applied: &AppliedHistoryEntry<'s>,
    dispatch_popstate: bool,
) {
    if runtime_window_is_global(scope, applied.owner) {
        let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
            return;
        };
        let host = unsafe { &mut *host_ptr };
        host.set_document_url(applied.parsed_url.clone());
        if dispatch_popstate && applied.previous_history_index != applied.history_index {
            host.record_same_document_navigation(
                &applied.parsed_url,
                "fragment",
                SameDocumentHistoryUpdate::Traverse {
                    delta: i64::from(applied.history_index)
                        - i64::from(applied.previous_history_index),
                },
            );
        }
        if dispatch_popstate {
            dispatch_popstate_event(scope, host_ptr, None, applied.state);
            perform_microtask_checkpoint_and_report_pending_promise_rejections(scope);
            queue_hash_change_for_runtime_owner(
                scope,
                applied.owner,
                applied.old_url.as_deref(),
                &applied.url,
            );
        }
    } else if dispatch_popstate && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        sync_local_document_front_from_window(scope, applied.owner);
        let child_handle =
            super::super::navigation_window::child_browsing_context_handle_for_runtime_owner(
                scope,
                applied.owner,
            );
        dispatch_popstate_event(scope, host_ptr, child_handle, applied.state);
        perform_microtask_checkpoint_and_report_pending_promise_rejections(scope);
        queue_hash_change_for_runtime_owner(
            scope,
            applied.owner,
            applied.old_url.as_deref(),
            &applied.url,
        );
        let entries_share_document = applied.previous_entry.is_some_and(|previous_entry| {
            navigation_entries_share_document(scope, previous_entry, applied.entry)
        });
        let is_same_document_traversal = entries_share_document
            || applied
                .old_url
                .as_deref()
                .and_then(|old_url| url::Url::parse(old_url).ok())
                .is_some_and(|old_url| {
                    is_same_document_fragment_navigation(Some(&old_url), &applied.parsed_url)
                });
        if let Some(child_handle) = child_handle
            && !is_same_document_traversal
        {
            unsafe { &mut *host_ptr }.queue_child_browsing_context_navigation_without_seed_update(
                child_handle,
                &applied.url,
                None,
            );
        }
    } else {
        sync_local_document_front_from_window(scope, applied.owner);
    }
}
