use super::location_runtime::is_same_document_fragment_navigation;
use super::navigation_activation::bind_navigation_entry_runtime_owner;
use super::navigation_entry::{
    create_navigation_entry, history_index, set_navigation_entry_document_id,
    stringify_history_state,
};
use super::navigation_serialize::{
    apply_current_document_referrer_policy_to_entry_snapshots, serialize_history_entries,
};
use super::navigation_window::{
    runtime_window_uses_top_level_history_model, window_history_for_holder,
};
use crate::native_bridge::NavigationHistoryEntrySeed;
use moli_page_types::{
    initial_navigation_history_seed as page_initial_navigation_history_seed,
    reload_navigation_seed, traversal_navigation_seed_candidate,
};

pub(super) fn initial_navigation_history_seed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    href: &str,
) -> NavigationHistoryEntrySeed {
    page_initial_navigation_history_seed(
        runtime_window_uses_top_level_history_model(scope, window),
        href,
    )
}

pub(super) fn build_history_entries_array_from_seed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    seed: &NavigationHistoryEntrySeed,
) -> v8::Local<'s, v8::Array> {
    let entries = v8::Array::new(scope, seed.entries.len() as i32);
    for snapshot in &seed.entries {
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
    }
    entries
}

pub(super) fn build_current_navigation_entry_from_seed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    seed: &NavigationHistoryEntrySeed,
    fallback_state: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Object> {
    let Some(snapshot) = seed
        .entries
        .iter()
        .find(|entry| entry.history_index == seed.current_index)
    else {
        let fallback_state_json = stringify_history_state(scope, fallback_state);
        let entry = create_navigation_entry(
            scope,
            "about:blank",
            fallback_state_json.as_deref(),
            fallback_state_json.as_deref(),
            None,
            0,
            "",
            "",
        );
        let document_id = moli_page_types::NavigationHistoryDocumentId::allocate();
        set_navigation_entry_document_id(scope, entry, document_id.as_str());
        bind_navigation_entry_runtime_owner(scope, entry, owner);
        return entry;
    };
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
    entry
}

pub(super) fn history_entry_seed_for_reload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<NavigationHistoryEntrySeed> {
    let history = window_history_for_holder(scope, owner)?;
    let entries = serialize_history_entries(scope, history);
    let current_index = history_index(scope, history);
    reload_navigation_seed(entries, current_index)
}

pub(super) fn history_entry_seed_for_traversal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    current_index: u32,
    target_index: u32,
) -> Option<(url::Url, NavigationHistoryEntrySeed)> {
    let history = window_history_for_holder(scope, owner)?;
    let mut entries = serialize_history_entries(scope, history);
    apply_current_document_referrer_policy_to_entry_snapshots(
        scope,
        owner,
        current_index,
        &mut entries,
    );
    let candidate = traversal_navigation_seed_candidate(entries, current_index, target_index)?;

    if is_same_document_fragment_navigation(Some(&candidate.current_url), &candidate.target_url) {
        return None;
    }

    Some((candidate.target_url, candidate.seed))
}
