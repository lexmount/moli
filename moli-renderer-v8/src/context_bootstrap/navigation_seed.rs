use super::history_runtime::native;
use super::navigation_entry::{history_index, navigation_entry_public_token};
use super::navigation_serialize::{
    apply_current_document_referrer_policy_to_entry_snapshots, serialize_history_entries,
};
use super::navigation_window::{
    runtime_window_uses_top_level_history_model, window_history_for_holder,
};
use crate::native_bridge::{NavigationHistoryEntrySeed, NavigationHistorySerializedEntry};
use crate::structured_clone::serialize_history_state;
use moli_history::{HistoryEntry, HistoryEntryRef, ScrollRestoration};
use moli_page_types::{
    NavigationHistoryEntryId,
    initial_navigation_history_seed as page_initial_navigation_history_seed,
    reload_navigation_seed, traversal_navigation_seed_candidate,
};
use moli_session_history::{NavigationHistoryDocumentId, NavigationHistoryEntryKey};

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

/// Restore the complete native record before exposing any JS wrapper.
pub(super) fn history_entry_from_snapshot(
    snapshot: &NavigationHistorySerializedEntry,
) -> HistoryEntryRef {
    HistoryEntry {
        url: snapshot.url.clone(),
        document_origin: snapshot.document_origin.clone(),
        referrer_policy: snapshot.referrer_policy.clone(),
        history_state: snapshot.history_state.clone(),
        navigation_state: snapshot.navigation_state.clone(),
        id: navigation_entry_public_token(&snapshot.id),
        key: NavigationHistoryEntryKey::from_serialized(navigation_entry_public_token(
            &snapshot.key,
        )),
        document: snapshot.document_id.clone(),
        index: snapshot.index,
        scroll_restoration: snapshot.scroll_restoration,
        scroll_offset: None,
    }
    .into_ref()
}

pub(super) fn build_history_entries_from_seed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    seed: &NavigationHistoryEntrySeed,
) -> Vec<HistoryEntryRef> {
    let mut snapshots: Vec<_> = seed.entries.iter().collect();
    snapshots.sort_by_key(|snapshot| snapshot.history_index);
    snapshots
        .into_iter()
        .map(|snapshot| {
            let entry = history_entry_from_snapshot(snapshot);
            if seed.entries.iter().find(|entry| entry.history_index == seed.current_index)
                .is_some_and(|current| current.document_id == snapshot.document_id)
            {
                entry.borrow_mut().document_origin = super::navigation_entry::navigation_document_origin(scope, owner, &snapshot.url);
            }
            entry
        })
        .collect()
}

pub(super) fn build_current_navigation_entry_from_seed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    seed: &NavigationHistoryEntrySeed,
    fallback_state: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Object> {
    let entry = seed
        .entries
        .iter()
        .find(|entry| entry.history_index == seed.current_index)
        .map(|snapshot| {
            let entry = history_entry_from_snapshot(snapshot);
            entry.borrow_mut().document_origin = super::navigation_entry::navigation_document_origin(scope, owner, &snapshot.url);
            entry
        })
        .unwrap_or_else(|| {
            let state = serialize_history_state(scope, fallback_state);
            HistoryEntry {
                url: "about:blank".to_owned(),
                document_origin: super::navigation_entry::navigation_document_origin(scope, owner, "about:blank"),
                referrer_policy: None,
                history_state: state.clone(),
                navigation_state: state,
                id: navigation_entry_public_token(NavigationHistoryEntryId::allocate().as_str()),
                key: NavigationHistoryEntryKey::from_serialized(navigation_entry_public_token(
                    NavigationHistoryEntryKey::allocate().as_str(),
                )),
                document: NavigationHistoryDocumentId::allocate(),
                index: 0,
                scroll_restoration: ScrollRestoration::Auto,
                scroll_offset: None,
            }
            .into_ref()
        });
    native::entry_wrapper(scope, owner, entry)
}

pub(super) fn history_entry_seed_for_reload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<NavigationHistoryEntrySeed> {
    let history = window_history_for_holder(scope, owner)?;
    let entries = serialize_history_entries(scope, history);
    let current_index = history_index(scope, history);
    let mut seed = reload_navigation_seed(entries, current_index)?;
    super::session_history::capture_for_navigation(scope, owner, &mut seed);
    Some(seed)
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
    let mut candidate = traversal_navigation_seed_candidate(entries, current_index, target_index)?;

    // The candidate already compares Document identities. Distinct Documents
    // can have identical URLs, including URLs that differ only in fragment.
    super::session_history::capture_for_navigation(scope, owner, &mut candidate.seed);
    Some((candidate.target_url, candidate.seed))
}
