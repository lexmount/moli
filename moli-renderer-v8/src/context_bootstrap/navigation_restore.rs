use super::navigation_activation::{
    bind_navigation_entry_runtime_owner, install_navigation_activation_runtime_state,
    set_navigation_current_entry,
};
use super::navigation_entry::{
    create_navigation_entry, set_history_entries, set_history_index, set_history_state,
    set_navigation_entry_document_id,
};
use super::navigation_entry_state::clone_history_entry_state;
use super::navigation_projection::set_history_length_from_visible_entries;
use super::navigation_result::clear_active_cross_document_navigation_if_matches;
use super::navigation_window::{window_history_for_holder, window_navigation_for_holder};
use crate::native_bridge::NavigationHistoryEntrySeed;

pub(crate) fn install_navigation_bootstrap_entry(
    scope: &mut v8::PinScope<'_, '_>,
    entry_seed: &NavigationHistoryEntrySeed,
) {
    let global = scope.get_current_context().global(scope);
    install_navigation_bootstrap_entry_for_holder(scope, global, entry_seed);
}

pub(crate) fn install_navigation_bootstrap_entry_for_holder<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    entry_seed: &NavigationHistoryEntrySeed,
) {
    let Some(history) = window_history_for_holder(scope, owner) else {
        return;
    };
    let Some(navigation) = window_navigation_for_holder(scope, owner) else {
        return;
    };
    let entries = v8::Array::new(scope, entry_seed.entries.len() as i32);
    let mut current_entry = None;
    let mut current_state: Option<v8::Local<'_, v8::Value>> = None;
    for snapshot in &entry_seed.entries {
        let entry = create_navigation_entry(
            scope,
            &snapshot.url,
            snapshot.history_state_json.as_deref(),
            snapshot.navigation_state_json.as_deref(),
            snapshot.referrer_policy.as_deref(),
            snapshot.index,
            &snapshot.id,
            &snapshot.key,
        );
        set_navigation_entry_document_id(scope, entry, snapshot.document_id.as_str());
        bind_navigation_entry_runtime_owner(scope, entry, owner);
        let _ = entries.set_index(scope, snapshot.history_index, entry.into());
        if snapshot.history_index == entry_seed.current_index {
            current_entry = Some(entry);
            current_state = Some(
                clone_history_entry_state(scope, entry).unwrap_or_else(|| v8::null(scope).into()),
            );
        }
    }
    let current_entry = current_entry.unwrap_or_else(|| {
        current_state = Some(v8::null(scope).into());
        let entry = create_navigation_entry(scope, "about:blank", None, None, None, 0, "", "");
        let document_id = moli_page_types::NavigationHistoryDocumentId::allocate();
        set_navigation_entry_document_id(scope, entry, document_id.as_str());
        bind_navigation_entry_runtime_owner(scope, entry, owner);
        entry
    });
    set_history_entries(scope, history, entries);
    set_history_index(scope, history, entry_seed.current_index);
    set_history_length_from_visible_entries(scope, history, entries);
    set_history_state(
        scope,
        history,
        current_state.unwrap_or_else(|| v8::null(scope).into()),
    );
    set_navigation_current_entry(scope, navigation, current_entry);
    if let Some(snapshot) = entry_seed
        .entries
        .iter()
        .find(|entry| entry.history_index == entry_seed.current_index)
    {
        clear_active_cross_document_navigation_if_matches(scope, navigation, &snapshot.url);
    }
    install_navigation_activation_runtime_state(
        scope,
        navigation,
        current_entry,
        entry_seed.activation.as_ref(),
    );
}
