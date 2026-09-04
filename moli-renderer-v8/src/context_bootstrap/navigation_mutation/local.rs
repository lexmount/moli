use super::*;

pub(crate) fn apply_local_window_location_navigation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    resolved: &url::Url,
    kind: LocationNavigationKind,
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

    match kind {
        LocationNavigationKind::Assign => {
            let next_index = current_index + 1;
            let next_navigation_index = current_navigation_index + 1;
            let next_entries = v8::Array::new(scope, (current_index + 2) as i32);
            for index in 0..=current_index {
                if let Some(entry) = entries.get_index(scope, index) {
                    let _ = next_entries.set_index(scope, index, entry);
                }
            }
            let state = v8::null(scope).into();
            let next_entry = create_navigation_entry(
                scope,
                resolved.as_str(),
                None,
                None,
                None,
                next_navigation_index,
                &new_navigation_entry_id(),
                &new_navigation_entry_key(),
            );
            let document_id = moli_page_types::NavigationHistoryDocumentId::allocate();
            set_navigation_entry_document_id(scope, next_entry, document_id.as_str());
            bind_navigation_entry_runtime_owner(scope, next_entry, owner);
            let _ = next_entries.set_index(scope, next_index, next_entry.into());
            set_history_entries(scope, history, next_entries);
            set_history_index(scope, history, next_index);
            // Cross-document pushes do not enter the joint session history
            // until the new Document commits. Keep the pending child's local
            // projection current without advancing the traversable yet.
            set_history_length_from_visible_entries(scope, history, next_entries);
            set_history_state(scope, history, state);
            set_navigation_current_entry(scope, navigation, next_entry);
            dispatch_navigation_currententrychange(scope, navigation, previous_entry, Some("push"));
        }
        LocationNavigationKind::Replace => {
            let state = v8::null(scope).into();
            let key = previous_entry
                .filter(|entry| replacement_keeps_navigation_key(scope, *entry, resolved))
                .and_then(|entry| navigation_entry_key_value(scope, entry))
                .unwrap_or_else(|| new_navigation_entry_key().as_str().to_owned());
            let entry = create_navigation_entry(
                scope,
                resolved.as_str(),
                None,
                None,
                None,
                current_navigation_index,
                &new_navigation_entry_id(),
                &key,
            );
            let document_id = moli_page_types::NavigationHistoryDocumentId::allocate();
            set_navigation_entry_document_id(scope, entry, document_id.as_str());
            bind_navigation_entry_runtime_owner(scope, entry, owner);
            let _ = entries.set_index(scope, current_index, entry.into());
            set_history_entries(scope, history, entries);
            set_history_state(scope, history, state);
            set_navigation_current_entry(scope, navigation, entry);
            dispatch_navigation_currententrychange(
                scope,
                navigation,
                previous_entry,
                Some("replace"),
            );
        }
        LocationNavigationKind::Reload => {
            if let Some(entry) = entries
                .get_index(scope, current_index)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            {
                let current_entry = previous_entry.unwrap_or(entry);
                if previous_entry.is_none() {
                    sync_navigation_current_entry_from_history_entry(scope, owner, entry);
                }
                let history_entry_snapshots = serialize_history_entries(scope, history);
                let current_entry_snapshot = serialize_navigation_entry_object(
                    scope,
                    current_entry,
                    &history_entry_snapshots,
                );
                let reload_activation = NavigationActivationSeed {
                    entry: current_entry_snapshot.clone(),
                    from: Some(current_entry_snapshot),
                    navigation_type: Some("reload".to_owned()),
                };
                install_navigation_activation_runtime_state(
                    scope,
                    navigation,
                    current_entry,
                    Some(&reload_activation),
                );
            }
        }
    }
    sync_child_pending_navigation_entry_seed_from_owner(scope, owner);
}

fn replacement_keeps_navigation_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    previous_entry: v8::Local<'s, v8::Object>,
    resolved: &url::Url,
) -> bool {
    navigation_entry_url_value(scope, previous_entry)
        .and_then(|url| url::Url::parse(&url).ok())
        .is_some_and(|previous_url| moli_url::same_origin(&previous_url, resolved))
}
