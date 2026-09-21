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
    let current_navigation_index = get_own_static_property(scope, current_entry, "index")
        .and_then(|value| value.integer_value(scope))
        .unwrap_or(0)
        .max(0) as u32;
    let current_index = history_index(scope, history);
    // Fragment entries share the Document and Navigation API state, but do
    // not inherit classic History API state from the previous entry.
    let history_state = v8::null(scope).into();
    let navigation_state = clone_navigation_entry_state(scope, current_entry);
    let navigation_state_json =
        navigation_state.and_then(|state| stringify_history_state(scope, state));
    let entries = history_entries(scope, history).unwrap_or_else(|| v8::Array::new(scope, 0));
    let mut pruned_entries = Vec::new();
    match kind {
        LocationNavigationKind::Assign => {
            let next_index = current_index + 1;
            let next_navigation_index = current_navigation_index + 1;
            pruned_entries = pruned_history_entries(scope, entries, next_index);
            let next_entry = create_navigation_entry(
                scope,
                owner,
                href,
                None,
                navigation_state_json.as_deref(),
                None,
                next_navigation_index,
                &new_navigation_entry_id(),
                &new_navigation_entry_key(),
            );
            copy_navigation_entry_document_id(scope, current_entry, next_entry);
            bind_navigation_entry_runtime_owner(scope, next_entry, owner);
            let next_entries = v8::Array::new(scope, (next_index + 1) as i32);
            for index in 0..next_index {
                if let Some(entry) = entries.get_index(scope, index) {
                    let _ = next_entries.set_index(scope, index, entry);
                }
            }
            let _ = next_entries.set_index(scope, next_index, next_entry.into());
            set_history_entries(scope, history, next_entries);
            set_history_index(scope, history, next_index);
            set_history_length_at_least_visible_entries(scope, history, next_entries);
            set_history_state(scope, history, history_state);
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
                navigation_state_json.as_deref(),
                None,
                current_navigation_index,
                &new_navigation_entry_id(),
                &key,
            );
            copy_navigation_entry_document_id(scope, current_entry, entry);
            bind_navigation_entry_runtime_owner(scope, entry, owner);
            let _ = entries.set_index(scope, current_index, entry.into());
            set_history_entries(scope, history, entries);
            set_history_state(scope, history, history_state);
            set_navigation_current_entry(scope, navigation, entry);
        }
        LocationNavigationKind::Reload => return,
    }
    sync_same_document_navigation_commit(
        scope,
        owner,
        href,
        protocol_navigation_type,
        Some(match kind {
            LocationNavigationKind::Assign => SameDocumentHistoryUpdate::Push,
            LocationNavigationKind::Replace => SameDocumentHistoryUpdate::Replace,
            LocationNavigationKind::Reload => return,
        }),
    );
    let navigation_type = match kind {
        LocationNavigationKind::Assign => Some("push"),
        LocationNavigationKind::Replace => Some("replace"),
        LocationNavigationKind::Reload => None,
    };
    dispatch_navigation_currententrychange(
        scope,
        navigation,
        Some(previous_entry),
        navigation_type,
    );
    dispatch_pruned_history_entry_disposes(scope, pruned_entries);
    sync_child_navigation_entry_seed_from_owner(scope, owner);
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
    let entries = history_entries(scope, history).unwrap_or_else(|| v8::Array::new(scope, 0));
    let current_index = history_index(scope, history);
    let current_navigation_index = navigation_current_entry_index(scope, owner).unwrap_or(0);
    let history_state = v8::null(scope).into();

    match kind {
        LocationNavigationKind::Assign => {
            let next_index = current_index + 1;
            let next_navigation_index = current_navigation_index + 1;
            let pruned_entries = pruned_history_entries(scope, entries, next_index);
            let next_entries = v8::Array::new(scope, (current_index + 2) as i32);
            for index in 0..=current_index {
                if let Some(entry) = entries.get_index(scope, index) {
                    let _ = next_entries.set_index(scope, index, entry);
                }
            }
            let next_entry = create_navigation_entry(
                scope,
                owner,
                href,
                None,
                None,
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
            let _ = next_entries.set_index(scope, next_index, next_entry.into());
            set_history_entries(scope, history, next_entries);
            set_history_index(scope, history, next_index);
            set_history_length_at_least_visible_entries(scope, history, next_entries);
            set_history_state(scope, history, history_state);
            set_navigation_current_entry(scope, navigation, next_entry);
            if let Some(location) = window_location_for_holder(scope, owner) {
                sync_location_object(scope, location, href);
            }
            sync_same_document_navigation_commit(
                scope,
                owner,
                href,
                protocol_navigation_type,
                Some(SameDocumentHistoryUpdate::Push),
            );
            dispatch_navigation_currententrychange(scope, navigation, previous_entry, Some("push"));
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
                None,
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
            let _ = entries.set_index(scope, current_index, entry.into());
            set_history_entries(scope, history, entries);
            set_history_state(scope, history, history_state);
            set_navigation_current_entry(scope, navigation, entry);
            if let Some(location) = window_location_for_holder(scope, owner) {
                sync_location_object(scope, location, href);
            }
            sync_same_document_navigation_commit(
                scope,
                owner,
                href,
                protocol_navigation_type,
                Some(SameDocumentHistoryUpdate::Replace),
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
    refresh_navigation_destination_indexes(scope, navigation, history);
    sync_child_navigation_entry_seed_from_owner(scope, owner);
}

fn pruned_history_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: v8::Local<'s, v8::Array>,
    first_pruned_index: u32,
) -> Vec<v8::Local<'s, v8::Object>> {
    (first_pruned_index..entries.length())
        .filter_map(|index| entries.get_index(scope, index))
        .filter_map(|entry| v8::Local::<v8::Object>::try_from(entry).ok())
        .collect()
}

fn dispatch_pruned_history_entry_disposes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: Vec<v8::Local<'s, v8::Object>>,
) {
    for entry in entries {
        dispatch_navigation_entry_dispose(scope, entry);
    }
}
