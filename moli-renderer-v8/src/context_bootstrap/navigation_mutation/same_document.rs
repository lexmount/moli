use super::super::history_runtime::state::{push_history_entry, replace_history_entry};
use super::*;

pub(in crate::context_bootstrap) fn update_navigation_current_entry_for_same_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    href: &str,
    kind: LocationNavigationKind,
    protocol_navigation_type: &str,
) {
    let Some(history) = window_history_for_holder(scope, owner) else {
        return;
    };
    let Some(navigation) = window_navigation_for_holder(scope, owner) else {
        return;
    };
    let Some(current_entry) = navigation_current_entry(scope, owner) else {
        return;
    };
    let previous_entry = current_entry;
    // The public index is relative to the exposed same-origin entries. A
    // restored history can contain hidden entries before that region, so use
    // the same native ordinal as History and Navigation API mutations.
    let current_navigation_index = navigation_current_entry_index(scope, owner).unwrap_or(0);
    let mut pruned_entries = Vec::new();
    match kind {
        LocationNavigationKind::Assign => {
            let next_navigation_index = current_navigation_index + 1;
            let next_entry = create_navigation_entry(
                scope,
                owner,
                href,
                None,
                next_navigation_index,
                &new_navigation_entry_id(),
                &new_navigation_entry_key(),
            );
            copy_navigation_entry_document_id(scope, current_entry, next_entry);
            copy_navigation_entry_serialized_state(scope, current_entry, next_entry);
            pruned_entries = push_history_entry(scope, history, next_entry);
            super::super::session_history::commit(
                scope,
                owner,
                next_entry,
                moli_page_types::SessionHistoryCommit::Push,
            );
            set_navigation_current_entry(scope, navigation, next_entry);
        }
        LocationNavigationKind::Replace => {
            let key = navigation_entry_key_value(scope, current_entry)
                .unwrap_or_else(|| new_navigation_entry_key().as_str().to_owned());
            let entry = create_navigation_entry(
                scope,
                owner,
                href,
                None,
                current_navigation_index,
                &new_navigation_entry_id(),
                &key,
            );
            copy_navigation_entry_document_id(scope, current_entry, entry);
            copy_navigation_entry_serialized_state(scope, current_entry, entry);
            replace_history_entry(scope, history, entry);
            set_navigation_current_entry(scope, navigation, entry);
            super::super::session_history::commit(
                scope,
                owner,
                entry,
                moli_page_types::SessionHistoryCommit::Replace,
            );
        }
        LocationNavigationKind::Reload => return,
    }
    sync_same_document_navigation_commit(scope, owner, href, protocol_navigation_type, true);
    let navigation_type = match kind {
        LocationNavigationKind::Assign => Some("push"),
        LocationNavigationKind::Replace => Some("replace"),
        LocationNavigationKind::Reload => None,
    };
    let joint_pruned = super::super::session_history::prune_views(scope, owner);
    dispatch_pruned_history_entry_disposes(scope, joint_pruned);
    dispatch_navigation_currententrychange(
        scope,
        navigation,
        Some(previous_entry),
        navigation_type,
    );
    dispatch_pruned_history_entry_disposes(scope, pruned_entries);
    sync_navigation_entry_seed_from_owner(scope, owner);
}

pub(in crate::context_bootstrap) fn apply_navigation_navigate_same_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    href: &str,
    kind: LocationNavigationKind,
    navigation_state: Option<v8::Local<'s, v8::Value>>,
    protocol_navigation_type: &str,
) {
    let Some(history) = window_history_for_holder(scope, owner) else {
        return;
    };
    let Some(navigation) = window_navigation_for_holder(scope, owner) else {
        return;
    };
    let previous_entry = navigation_current_entry(scope, owner);
    let current_navigation_index = navigation_current_entry_index(scope, owner).unwrap_or(0);
    let history_state = v8::null(scope).into();

    match kind {
        LocationNavigationKind::Assign => {
            let next_navigation_index = current_navigation_index + 1;
            let next_entry = create_navigation_entry(
                scope,
                owner,
                href,
                None,
                next_navigation_index,
                &new_navigation_entry_id(),
                &new_navigation_entry_key(),
            );
            if let Some(previous_entry) = previous_entry {
                copy_navigation_entry_document_id(scope, previous_entry, next_entry);
            }
            bind_navigation_entry_runtime_owner(scope, next_entry, owner);
            if let Some(state) = navigation_state {
                set_navigation_entry_state(scope, next_entry, state);
            }
            let pruned_entries = push_history_entry(scope, history, next_entry);
            super::super::session_history::commit(
                scope,
                owner,
                next_entry,
                moli_page_types::SessionHistoryCommit::Push,
            );
            cache_current_history_state(scope, history, history_state);
            set_navigation_current_entry(scope, navigation, next_entry);
            if let Some(location) = window_location_for_holder(scope, owner) {
                sync_location_object(scope, location, href);
            }
            let joint_pruned = super::super::session_history::prune_views(scope, owner);
            sync_same_document_navigation_commit(
                scope,
                owner,
                href,
                protocol_navigation_type,
                true,
            );
            dispatch_navigation_currententrychange(scope, navigation, previous_entry, Some("push"));
            dispatch_pruned_history_entry_disposes(scope, joint_pruned);
            dispatch_pruned_history_entry_disposes(scope, pruned_entries);
        }
        LocationNavigationKind::Replace => {
            let key = previous_entry
                .and_then(|entry| navigation_entry_key_value(scope, entry))
                .unwrap_or_else(|| new_navigation_entry_key().as_str().to_owned());
            let entry = create_navigation_entry(
                scope,
                owner,
                href,
                None,
                current_navigation_index,
                &new_navigation_entry_id(),
                &key,
            );
            if let Some(previous_entry) = previous_entry {
                copy_navigation_entry_document_id(scope, previous_entry, entry);
            }
            bind_navigation_entry_runtime_owner(scope, entry, owner);
            if let Some(state) = navigation_state {
                set_navigation_entry_state(scope, entry, state);
            }
            replace_history_entry(scope, history, entry);
            cache_current_history_state(scope, history, history_state);
            set_navigation_current_entry(scope, navigation, entry);
            super::super::session_history::commit(
                scope,
                owner,
                entry,
                moli_page_types::SessionHistoryCommit::Replace,
            );
            if let Some(location) = window_location_for_holder(scope, owner) {
                sync_location_object(scope, location, href);
            }
            sync_same_document_navigation_commit(
                scope,
                owner,
                href,
                protocol_navigation_type,
                true,
            );
            dispatch_navigation_currententrychange(
                scope,
                navigation,
                previous_entry,
                Some("replace"),
            );
            if let Some(previous_entry) = previous_entry {
                dispatch_navigation_entry_dispose(scope, previous_entry);
            }
        }
        LocationNavigationKind::Reload => return,
    }
    sync_navigation_entry_seed_from_owner(scope, owner);
}

fn dispatch_pruned_history_entry_disposes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: Vec<v8::Local<'s, v8::Object>>,
) {
    for entry in entries {
        dispatch_navigation_entry_dispose(scope, entry);
    }
}
