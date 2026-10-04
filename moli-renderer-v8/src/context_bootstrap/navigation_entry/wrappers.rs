//! Complete NavigationHistoryEntry wrappers. Runtime ownership and creation
//! realm are independent: Window wrappers use the owner's realm and strong
//! cache; other worlds share the native record and event target via weak views.

use std::{collections::HashMap, rc::Rc};

use moli_history::HistoryEntryRef;

use super::NavigationHistoryEntryObjectDeclaration;
use crate::context_bootstrap::history_runtime::native;
use crate::context_bootstrap::location_history_storage::NAVIGATION_ENTRY_EVENT_LISTENERS_SLOT;
use crate::context_bootstrap::navigation_window::{runtime_window_owner, set_runtime_window_owner};
use crate::context_bootstrap::{media_queries, shared_event_targets};
use crate::util::{get_private_value, set_private_value, v8_string};

const ENTRY_WRAPPERS: &str = "__moliNavigationEntryWrappers";

/// Get the canonical wrapper in the owning Window's realm. The entire cache
/// operation runs there, even when first called from another Window or world.
pub(in crate::context_bootstrap) fn for_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    record: HistoryEntryRef,
) -> v8::Local<'s, v8::Object> {
    let context = owner
        .get_creation_context(scope)
        .expect("History Window realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    let id = record.borrow().id.clone();
    let key = v8_string(scope, &id);
    let map = window_wrappers(scope, owner);
    if let Some(key) = key
        && let Some(wrapper) = map
            .get(scope, key.into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        && native::entry(scope, wrapper).is_some_and(|entry| Rc::ptr_eq(&entry, &record))
    {
        return wrapper;
    }
    let wrapper = allocate_in_current_realm(scope, owner, record);
    if let Some(key) = key {
        let _ = map.set(scope, key.into(), wrapper.into());
    }
    wrapper
}

/// Preserve the source's runtime owner and shared EventTarget, while using the
/// requested realm's interface. Weak reuse never roots that realm in Window.
pub(in crate::context_bootstrap) fn in_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Context>,
) -> v8::Local<'s, v8::Object> {
    if source.get_creation_context(scope) == Some(context) {
        return source;
    }
    let Some(record) = native::entry(scope, source) else {
        return source;
    };
    if let Some(wrapper) = native::find_entry_wrapper(scope, &record, context) {
        return wrapper;
    }
    let owner = runtime_window_owner(scope, source);
    let target = shared_event_targets::shared_target_owner(scope, source);
    let scope = &mut v8::ContextScope::new(scope, context);
    let wrapper = allocate_in_current_realm(scope, owner, record);
    shared_event_targets::bind_shared_target(scope, wrapper, target);
    wrapper
}

pub(in crate::context_bootstrap) fn value_in_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: v8::Local<'s, v8::Context>,
) -> v8::Local<'s, v8::Value> {
    match v8::Local::<v8::Object>::try_from(value) {
        Ok(object) => in_realm(scope, object, context).into(),
        Err(_) => value,
    }
}

pub(in crate::context_bootstrap) fn prune_for_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    entries: &[HistoryEntryRef],
) {
    let live: HashMap<_, _> = entries
        .iter()
        .map(|entry| (entry.borrow().id.clone(), Rc::as_ptr(entry)))
        .collect();
    let Some(map) = get_private_value(scope, owner, ENTRY_WRAPPERS)
        .and_then(|value| v8::Local::<v8::Map>::try_from(value).ok())
    else {
        return;
    };
    let pairs = map.as_array(scope);
    for index in (0..pairs.length()).step_by(2) {
        let Some(key) = pairs.get_index(scope, index) else {
            continue;
        };
        let record = pairs
            .get_index(scope, index + 1)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .and_then(|wrapper| native::entry(scope, wrapper));
        let retained = record
            .is_some_and(|record| live.get(&record.borrow().id) == Some(&Rc::as_ptr(&record)));
        if !retained {
            let _ = map.delete(scope, key);
        }
    }
}

// The caller selects the creation realm. Owner binding must never switch it.
fn allocate_in_current_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    record: HistoryEntryRef,
) -> v8::Local<'s, v8::Object> {
    let wrapper = NavigationHistoryEntryObjectDeclaration::new()
        .bind(scope)
        .expect("NavigationHistoryEntry declaration should bind");
    native::bind_entry(scope, wrapper, record);
    media_queries::install_simple_event_target_methods(
        scope,
        wrapper,
        NAVIGATION_ENTRY_EVENT_LISTENERS_SLOT,
        false,
    );
    shared_event_targets::install_handlers(scope, wrapper, true);
    set_runtime_window_owner(scope, wrapper, owner);
    wrapper
}

fn window_wrappers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Map> {
    if let Some(map) = get_private_value(scope, owner, ENTRY_WRAPPERS)
        .and_then(|value| v8::Local::<v8::Map>::try_from(value).ok())
    {
        return map;
    }
    let map = v8::Map::new(scope);
    set_private_value(scope, owner, ENTRY_WRAPPERS, map.into());
    map
}
