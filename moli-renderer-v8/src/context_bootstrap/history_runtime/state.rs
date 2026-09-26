//! Native History state with realm-local caches of deserialized JS values.

use super::super::navigation_window::runtime_window_owner;
use super::native;
use crate::structured_clone::deserialize_history_state;
use crate::util::{get_private_value, private_key, set_private_value, v8_string};
use moli_history::{HistoryEntryRef, ScrollRestoration};

const WINDOW_HISTORY_OWNER_SLOT: &str = "__moliWindowHistoryOwner";
const CACHED_REVISION_SLOT: &str = "__moliHistoryCachedRevision";
const CACHED_STATE_SLOT: &str = "__moliHistoryCachedState";

/// Bind once during realm creation; never resolve an old wrapper to a new
/// document's current Window by looking up its frame at call time.
pub(crate) fn bind_isolated_window_history_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    owner: v8::Local<'s, v8::Object>,
) {
    set_private_value(scope, window, WINDOW_HISTORY_OWNER_SLOT, owner.into());
}

pub(in crate::context_bootstrap) fn history_window_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    get_private_value(scope, window, WINDOW_HISTORY_OWNER_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .unwrap_or(window)
}

pub(in crate::context_bootstrap) fn window_has_shared_history<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> bool {
    !history_window_owner(scope, window).strict_equals(window.into())
}

pub(in crate::context_bootstrap) fn history_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
) -> Option<Vec<HistoryEntryRef>> {
    let record = native::history(scope, history)?;
    let entries = record.borrow().entries().to_vec();
    Some(entries)
}

pub(in crate::context_bootstrap) fn set_history_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    entries: Vec<HistoryEntryRef>,
) {
    let Some(record) = native::history(scope, history) else {
        return;
    };
    let owner = runtime_window_owner(scope, history);
    native::prune_entry_wrappers(scope, owner, &entries);
    record.borrow_mut().restore_entries(entries);
}

pub(in crate::context_bootstrap) fn push_history_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    wrapper: v8::Local<'s, v8::Object>,
) -> Vec<v8::Local<'s, v8::Object>> {
    let Some(record) = native::history(scope, history) else {
        return Vec::new();
    };
    let Some(entry) = native::entry(scope, wrapper) else {
        return Vec::new();
    };
    let owner = runtime_window_owner(scope, history);
    super::super::navigation_activation::bind_navigation_entry_runtime_owner(scope, wrapper, owner);
    let removed = record.borrow_mut().push(entry);
    let removed = removed
        .into_iter()
        .map(|entry| native::entry_wrapper(scope, owner, entry))
        .collect();
    let retained = record.borrow().entries().to_vec();
    native::prune_entry_wrappers(scope, owner, &retained);
    removed
}

pub(in crate::context_bootstrap) fn replace_history_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    wrapper: v8::Local<'s, v8::Object>,
) {
    let Some(record) = native::history(scope, history) else {
        return;
    };
    let Some(entry) = native::entry(scope, wrapper) else {
        return;
    };
    let owner = runtime_window_owner(scope, history);
    super::super::navigation_activation::bind_navigation_entry_runtime_owner(scope, wrapper, owner);
    record.borrow_mut().replace(entry);
    let retained = record.borrow().entries().to_vec();
    native::prune_entry_wrappers(scope, owner, &retained);
}

pub(in crate::context_bootstrap) fn history_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
) -> u32 {
    native::history(scope, history).map_or(0, |record| record.borrow().current_index())
}

pub(in crate::context_bootstrap) fn set_history_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    index: u32,
) {
    if let Some(record) = native::history(scope, history) {
        record.borrow_mut().set_current_index(index);
    }
}

pub(in crate::context_bootstrap) fn history_scroll_restoration_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let record = native::history(scope, history)?;
    let value = record.borrow().scroll_restoration().as_str();
    v8_string(scope, value).map(Into::into)
}

pub(in crate::context_bootstrap) fn set_history_scroll_restoration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    value: &str,
) {
    if let Some(record) = native::history(scope, history) {
        record.borrow_mut().set_scroll_restoration(match value {
            "manual" => ScrollRestoration::Manual,
            _ => ScrollRestoration::Auto,
        });
    }
}

pub(in crate::context_bootstrap) fn history_state_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Value> {
    let Some(record) = native::history(scope, history) else {
        return v8::null(scope).into();
    };
    let (revision, snapshot) = {
        let record = record.borrow();
        (record.revision(), record.state())
    };
    let cached_revision = get_private_value(scope, history, CACHED_REVISION_SLOT)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .map(|value| value.u64_value().0);
    if cached_revision == Some(revision)
        && let Some(key) = private_key(scope, CACHED_STATE_SLOT)
        && let Some(value) = history.get_private(scope, key)
    {
        return value;
    }
    let Some(context) = history.get_creation_context(scope) else {
        return v8::null(scope).into();
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let value = snapshot
        .as_ref()
        .and_then(|state| deserialize_history_state(scope, state))
        .unwrap_or_else(|| v8::null(scope).into());
    cache_history_state(scope, history, revision, value);
    value
}

fn cache_history_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    revision: u64,
    value: v8::Local<'s, v8::Value>,
) {
    set_private_value(
        scope,
        history,
        CACHED_REVISION_SLOT,
        v8::BigInt::new_from_u64(scope, revision).into(),
    );
    set_private_value(scope, history, CACHED_STATE_SLOT, value);
}

/// Cache an already deserialized value after a native history transition.
/// The entry's serialized state stays authoritative and is never rewritten
/// from a mutable JS cache during traversal.
pub(in crate::context_bootstrap) fn cache_current_history_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    state: v8::Local<'s, v8::Value>,
) {
    let Some(record) = native::history(scope, history) else {
        return;
    };
    let revision = record.borrow().revision();
    let history_context = history.get_creation_context(scope);
    let state_context = v8::Local::<v8::Object>::try_from(state)
        .ok()
        .map_or(history_context, |state| state.get_creation_context(scope));
    if state_context == history_context {
        cache_history_state(scope, history, revision, state);
    }
}
