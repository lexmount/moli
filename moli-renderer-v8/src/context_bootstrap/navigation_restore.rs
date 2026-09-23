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
use crate::context_bootstrap::navigation_entry::wrappers as entry_wrappers;
use crate::native_bridge::NavigationHistoryEntrySeed;

/// Preserve committed inherited-origin entries when a browser navigation
/// replaces their Document. The browser's position determines the history
/// operation; unsupported joint-history positions keep the existing path.
pub(crate) fn capture_inherited_history_for_browser_commit(
    scope: &mut v8::PinScope<'_, '_>,
    url: &url::Url,
    position: moli_session_history::SessionHistoryPosition,
) -> Option<NavigationHistoryEntrySeed> {
    let owner = scope.get_current_context().global(scope);
    let history = window_history_for_holder(scope, owner)?;
    let entries = super::navigation_serialize::serialize_history_entries(scope, history);
    if !entries.iter().any(|entry| {
        entry
            .inherited_origin
            .as_deref()
            .is_some_and(|origin| origin != "null")
    }) {
        return None;
    }
    let current = super::navigation_entry::history_index(scope, history);
    let target = u32::try_from(position.index()).ok()?;
    let mut seed = if position.length() == entries.len()
        && entries
            .iter()
            .any(|entry| entry.history_index == target && entry.url == url.as_str())
    {
        if current == target {
            moli_page_types::reload_navigation_seed(entries, current)?
        } else {
            moli_page_types::traversal_navigation_seed_candidate(entries, current, target)?.seed
        }
    } else {
        let mutation = if target == current && position.length() == entries.len() {
            moli_page_types::NavigationHistoryMutation::Replace
        } else if target == current.checked_add(1)?
            && position.length() == position.index().checked_add(1)?
        {
            moli_page_types::NavigationHistoryMutation::Push
        } else {
            return None;
        };
        let navigation_index =
            super::navigation_entry::navigation_current_entry_index(scope, owner).unwrap_or(0);
        moli_page_types::cross_document_navigation_seed(
            entries,
            current,
            navigation_index,
            url,
            mutation,
        )
    };
    super::session_history::capture_for_navigation(scope, owner, &mut seed);
    Some(seed)
}

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
    let entries = build_history_entries_from_seed(entry_seed);
    let current_entry = entries
        .get(entry_seed.current_index as usize)
        .map(|entry| entry_wrappers::for_window(scope, owner, entry.clone()))
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
