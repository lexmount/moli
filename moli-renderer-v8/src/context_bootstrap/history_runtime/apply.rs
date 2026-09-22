use super::super::location_runtime::{
    is_same_document_fragment_navigation, location_href_slot, sync_location_object,
};
use super::super::navigation_entry::{
    history_entries, history_index, navigation_current_entry, navigation_entries_share_document,
    navigation_entry_key_value, navigation_entry_url_value,
    restore_current_navigation_entry_scroll_position, set_history_index, set_history_state,
    sync_navigation_current_entry_from_history_entry,
};
use super::super::navigation_events::{
    dispatch_navigation_currententrychange, dispatch_navigation_success, dispatch_popstate_event,
    navigation_has_active_scroll_event, queue_hash_change_for_runtime_owner,
};
use super::super::navigation_mutation::sync_same_document_navigation_commit;
use super::super::navigation_result::perform_navigation_scroll_if_needed;
use super::super::navigation_serialize::sync_navigation_entry_seed_from_owner;
use super::super::navigation_window::{
    navigation_document_has_opaque_origin, runtime_window_is_global, runtime_window_owner,
    window_location_for_holder, window_navigation_for_holder, window_task_target_for_runtime_owner,
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
    let Some(applied) = apply_history_entry_commit(
        scope,
        history,
        index,
        dispatch_popstate.then_some("fragment"),
    ) else {
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
    protocol_navigation_type: Option<&str>,
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
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let host = unsafe { &mut *host_ptr };
        if let Some(target) = window_task_target_for_runtime_owner(scope, host, owner) {
            let entry_key = navigation_entry_key_value(scope, entry);
            host.commit_active_history_delta_position(target, index, entry_key);
        }
    }
    set_history_state(scope, history, state);

    let location = window_location_for_holder(scope, owner)?;
    sync_location_object(scope, location, &url);
    let Ok(parsed_url) = url::Url::parse(&url) else {
        return None;
    };
    sync_navigation_current_entry_from_history_entry(scope, owner, entry);
    let resolved_entry = navigation_current_entry(scope, owner)
        .map(v8::Local::<v8::Value>::from)
        .unwrap_or_else(|| v8::undefined(scope).into());
    sync_navigation_entry_seed_from_owner(scope, owner);
    sync_same_document_navigation_commit(
        scope,
        owner,
        &url,
        protocol_navigation_type.unwrap_or("fragment"),
        (protocol_navigation_type.is_some() && previous_history_index != index).then_some(
            SameDocumentHistoryUpdate::Traverse {
                delta: i64::from(index) - i64::from(previous_history_index),
            },
        ),
    );
    Some(AppliedHistoryEntry {
        owner,
        state,
        old_url,
        url,
        parsed_url,
        resolved_entry,
        entry,
        previous_entry,
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
        if dispatch_popstate {
            dispatch_popstate_event(scope, host_ptr, applied.owner, applied.state);
            perform_microtask_checkpoint_and_report_pending_promise_rejections(scope);
            queue_hash_change_for_runtime_owner(
                scope,
                applied.owner,
                applied.old_url.as_deref(),
                &applied.url,
            );
        }
    } else if dispatch_popstate && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        dispatch_popstate_event(scope, host_ptr, applied.owner, applied.state);
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
        if let Some(child_handle) =
            super::super::navigation_window::child_browsing_context_handle_for_runtime_owner(
                scope,
                applied.owner,
            )
            && !is_same_document_traversal
        {
            unsafe { &mut *host_ptr }.queue_child_browsing_context_navigation_without_seed_update(
                child_handle,
                &applied.url,
                None,
            );
        }
    }
}
