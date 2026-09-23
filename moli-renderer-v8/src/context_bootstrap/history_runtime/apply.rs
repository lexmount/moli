use super::super::location_runtime::{
    is_same_document_fragment_navigation, location_href_slot, sync_location_object,
};
use super::super::navigation_entry::{
    history_entries, history_index, navigation_current_entry, navigation_entries_share_document,
    navigation_entry_url_value, set_history_index, set_history_state,
    sync_navigation_current_entry_from_history_entry,
};
use super::super::navigation_events::{
    dispatch_navigation_currententrychange, dispatch_popstate_event,
    queue_hash_change_for_runtime_owner,
};
use super::super::navigation_mutation::sync_local_document_front_from_window;
use super::super::navigation_serialize::sync_child_navigation_entry_seed_from_owner;
use super::super::navigation_window::{
    navigation_document_has_opaque_origin, runtime_window_is_global, runtime_window_owner,
    window_location_for_holder, window_navigation_for_holder,
};
use super::super::*;
use crate::script_vm::perform_microtask_checkpoint_and_report_pending_promise_rejections;

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

pub(in crate::context_bootstrap) struct PreparedHistoryEntry<'s> {
    history: v8::Local<'s, v8::Object>,
    location: v8::Local<'s, v8::Object>,
    applied: AppliedHistoryEntry<'s>,
}

/// Resolve every fallible input before any participant changes its live view.
pub(in crate::context_bootstrap) fn prepare_local_history_entry_commit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    index: u32,
) -> Option<PreparedHistoryEntry<'s>> {
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
    let location = window_location_for_holder(scope, owner)?;
    let parsed_url = url::Url::parse(&url).ok()?;
    Some(PreparedHistoryEntry {
        history,
        location,
        applied: AppliedHistoryEntry {
            owner,
            state,
            old_url,
            url,
            parsed_url,
            resolved_entry: v8::undefined(scope).into(),
            previous_history_index,
            history_index: index,
            entry,
            previous_entry,
        },
    })
}

/// No author callbacks or fallible lookups are allowed in the commit phase.
pub(in crate::context_bootstrap) fn commit_prepared_history_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prepared: PreparedHistoryEntry<'s>,
) -> AppliedHistoryEntry<'s> {
    let PreparedHistoryEntry {
        history,
        location,
        mut applied,
    } = prepared;
    set_history_index(scope, history, applied.history_index);
    set_history_state(scope, history, applied.state);
    sync_location_object(scope, location, &applied.url);
    sync_navigation_current_entry_from_history_entry(scope, applied.owner, applied.entry);
    applied.resolved_entry = navigation_current_entry(scope, applied.owner)
        .map(Into::into)
        .unwrap_or_else(|| v8::undefined(scope).into());
    sync_child_navigation_entry_seed_from_owner(scope, applied.owner);
    applied
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
            host.record_same_document_navigation(&applied.parsed_url, "fragment");
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
            );
        }
    } else {
        sync_local_document_front_from_window(scope, applied.owner);
    }
}
