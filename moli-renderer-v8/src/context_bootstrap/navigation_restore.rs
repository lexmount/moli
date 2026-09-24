use super::history_runtime::native;
use super::navigation_activation::{
    install_navigation_activation_runtime_state, set_navigation_current_entry,
};
use super::navigation_entry::{
    cache_current_history_state, set_history_entries, set_history_index,
};
use super::navigation_entry_state::clone_history_entry_state;
use super::navigation_result::clear_active_cross_document_navigation_if_matches;
use super::navigation_seed::{
    build_current_navigation_entry_from_seed, build_history_entries_from_seed,
};
use super::navigation_window::{navigation_has_current_document, window_history_for_holder, window_navigation_for_holder};
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
    if super::history_runtime::state::window_has_shared_history(scope, owner) {
        return;
    }
    install_navigation_entry_view_for_holder(scope, owner, entry_seed);
    commit_navigation_history_for_document(scope, owner, entry_seed);
}

/// Document installation is the authoritative boundary, including commits
/// that reuse an initial Window and therefore do not initialize a new realm.
pub(crate) fn commit_navigation_history_for_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    entry_seed: &NavigationHistoryEntrySeed,
) {
    super::session_history::initialize(scope, owner, entry_seed);
    super::session_history::restore(scope, owner, entry_seed);
}

/// Refresh a Document's projection without mutating the shared traversable.
/// During an accepted cross-Document traversal this can still be the outgoing
/// Document's view. Only document bootstrap or an explicit history operation
/// may commit a session history transition.
pub(crate) fn install_navigation_entry_view_for_holder<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    entry_seed: &NavigationHistoryEntrySeed,
) {
    if super::history_runtime::state::window_has_shared_history(scope, owner) {
        return;
    }
    if let Some(navigation) = window_navigation_for_holder(scope, owner)
        && !navigation_has_current_document(scope, navigation)
    {
        let Some(current) = entry_seed
            .entries
            .iter()
            .find(|entry| entry.history_index == entry_seed.current_index)
        else {
            return;
        };
        // A javascript: navigation can replace the Document without changing
        // its URL. Do not install the new seed into the old Navigation object.
        if super::navigation_bootstrap::reset_window_location_history_navigation_runtime_state(
            scope,
            owner,
            &current.url,
        )
        .is_err()
        {
            return;
        }
    }
    let Some(history) = window_history_for_holder(scope, owner) else {
        return;
    };
    let Some(navigation) = window_navigation_for_holder(scope, owner) else {
        return;
    };
    let entries = build_history_entries_from_seed(scope, owner, entry_seed);
    let current_entry = entries
        .get(entry_seed.current_index as usize)
        .map(|entry| native::entry_wrapper(scope, owner, entry.clone()))
        .unwrap_or_else(|| {
            build_current_navigation_entry_from_seed(
                scope,
                owner,
                entry_seed,
                v8::null(scope).into(),
            )
        });
    let current_state =
        clone_history_entry_state(scope, current_entry).unwrap_or_else(|| v8::null(scope).into());
    set_history_entries(scope, history, entries);
    set_history_index(scope, history, entry_seed.current_index);
    cache_current_history_state(scope, history, current_state);
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
    if let Some(entry) = &entry_seed.session_history.admitted_entry {
        // An accepted child load may reuse a Window whose Document committed
        // before this projection was refreshed. Prune only the installed view.
        super::session_history_traversal::finish_entry(scope, owner, Some(&entry.key));
    }
    super::navigation_serialize::publish_top_level_navigation_history(scope, owner);
}
