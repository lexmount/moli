//! One History implementation per Window, with realm-local public wrappers.
//!
//! The private backing object keeps V8 payloads in the GC graph. Wrappers share
//! it rather than copying entry identities, the cursor, or restoration policy.
//! Only the deserialized `history.state` cache belongs to a particular wrapper.

use super::super::navigation_window::window_history_for_holder;
use super::super::structured_clone_value;
use crate::util::{get_private_value, private_key, set_private_value, v8_string};
use moli_webapi_declare::WebApiObject;

pub(in crate::context_bootstrap) const HISTORY_BACKING_SLOT: &str = "__moliHistoryBacking";
const WINDOW_HISTORY_OWNER_SLOT: &str = "__moliWindowHistoryOwner";
const ENTRIES_SLOT: &str = "entries";
const INDEX_SLOT: &str = "index";
const STATE_SLOT: &str = "state";
const SCROLL_RESTORATION_SLOT: &str = "scrollRestoration";
const CACHED_SNAPSHOT_SLOT: &str = "__moliHistoryCachedSnapshot";
const CACHED_STATE_SLOT: &str = "__moliHistoryCachedState";

#[derive(WebApiObject)]
#[webapi(plain)]
struct HistoryBackingDeclaration<'scope> {
    #[webapi(slot = ENTRIES_SLOT)]
    entries: v8::Local<'scope, v8::Array>,
    #[webapi(slot = INDEX_SLOT)]
    index: f64,
    #[webapi(slot = STATE_SLOT)]
    state: v8::Local<'scope, v8::Value>,
    #[webapi(slot = SCROLL_RESTORATION_SLOT)]
    scroll_restoration: &'static str,
}

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

pub(in crate::context_bootstrap) fn shared_history_backing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if !window_has_shared_history(scope, window) {
        return None;
    }
    let owner = history_window_owner(scope, window);
    let history = window_history_for_holder(scope, owner)?;
    backing(scope, history)
}

pub(in crate::context_bootstrap) fn new_history_backing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: v8::Local<'s, v8::Array>,
    index: u32,
) -> v8::Local<'s, v8::Object> {
    HistoryBackingDeclaration::new(entries, index as f64, v8::null(scope).into(), "auto")
        .bind(scope)
        .expect("History backing declaration should bind")
}

fn backing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, history, HISTORY_BACKING_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(in crate::context_bootstrap) fn history_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    let backing = backing(scope, history)?;
    get_private_value(scope, backing, ENTRIES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

pub(in crate::context_bootstrap) fn set_history_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    entries: v8::Local<'s, v8::Array>,
) {
    if let Some(backing) = backing(scope, history) {
        set_private_value(scope, backing, ENTRIES_SLOT, entries.into());
    }
}

pub(in crate::context_bootstrap) fn history_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
) -> u32 {
    backing(scope, history)
        .and_then(|backing| get_private_value(scope, backing, INDEX_SLOT))
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0)
}

pub(in crate::context_bootstrap) fn set_history_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    index: u32,
) {
    if let Some(backing) = backing(scope, history) {
        set_private_value(
            scope,
            backing,
            INDEX_SLOT,
            v8::Integer::new_from_unsigned(scope, index).into(),
        );
    }
}

pub(in crate::context_bootstrap) fn history_scroll_restoration_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let backing = backing(scope, history)?;
    get_private_value(scope, backing, SCROLL_RESTORATION_SLOT)
}

pub(in crate::context_bootstrap) fn set_history_scroll_restoration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    value: &str,
) {
    if let Some(backing) = backing(scope, history)
        && let Some(value) = v8_string(scope, value)
    {
        set_private_value(scope, backing, SCROLL_RESTORATION_SLOT, value.into());
    }
}

pub(in crate::context_bootstrap) fn history_state_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Value> {
    let snapshot = backing(scope, history)
        .and_then(|backing| {
            let key = private_key(scope, STATE_SLOT)?;
            backing.get_private(scope, key)
        })
        .unwrap_or_else(|| v8::null(scope).into());
    if !snapshot.is_object() {
        return snapshot;
    }
    if get_private_value(scope, history, CACHED_SNAPSHOT_SLOT)
        .is_some_and(|cached| cached.same_value(snapshot))
        && let Some(value) = get_private_value(scope, history, CACHED_STATE_SLOT)
    {
        return value;
    }
    let Some(context) = history.get_creation_context(scope) else {
        return v8::null(scope).into();
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let value = structured_clone_value(scope, snapshot).unwrap_or_else(|| v8::null(scope).into());
    set_private_value(scope, history, CACHED_SNAPSHOT_SLOT, snapshot);
    set_private_value(scope, history, CACHED_STATE_SLOT, value);
    value
}

pub(in crate::context_bootstrap) fn set_history_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    state: v8::Local<'s, v8::Value>,
) {
    let Some(backing) = backing(scope, history) else {
        return;
    };
    // The cache can be mutated by script (and is also popstate.state). Keep a
    // separate snapshot so another realm never deserializes those mutations.
    let snapshot = structured_clone_value(scope, state).unwrap_or_else(|| v8::null(scope).into());
    set_private_value(scope, backing, STATE_SLOT, snapshot);
    if history.get_creation_context(scope) == Some(scope.get_current_context()) {
        set_private_value(scope, history, CACHED_SNAPSHOT_SLOT, snapshot);
        set_private_value(scope, history, CACHED_STATE_SLOT, state);
    }
}
