//! Realm wrappers lease native records. The registry holds only weak V8
//! handles, so a wrapper/Window cycle cannot keep a retired history alive.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use moli_history::{HistoryEntryRef, WindowHistory};

use crate::util::{get_private_value, set_private_value, v8_string};

pub(in crate::context_bootstrap) type HistoryRef = Rc<RefCell<WindowHistory>>;
type Store = Rc<RefCell<NativeRecords>>;
const HISTORY_ID: &str = "__moliNativeHistory";
const ENTRY_ID: &str = "__moliNativeHistoryEntry";
const ENTRY_WRAPPERS: &str = "__moliNavigationEntryWrappers";

#[derive(Default)]
struct NativeRecords {
    next_id: u64,
    histories: HashMap<u64, (v8::Weak<v8::Object>, HistoryRef)>,
    entries: HashMap<u64, (Rc<v8::Weak<v8::Object>>, HistoryEntryRef)>,
    entry_ids: HashMap<usize, Vec<u64>>,
}

fn store(scope: &mut v8::PinScope<'_, '_>) -> Store {
    if let Some(store) = scope.get_slot::<Store>() {
        return store.clone();
    }
    let store = Store::default();
    scope.set_slot(store.clone());
    store
}

fn next_id(store: &Store) -> u64 {
    let mut store = store.borrow_mut();
    store.next_id = store
        .next_id
        .checked_add(1)
        .expect("native history identity exhausted");
    store.next_id
}

fn id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<u64> {
    let value = get_private_value(scope, object, slot)?;
    let value = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (id, lossless) = value.u64_value();
    lossless.then_some(id)
}

pub(in crate::context_bootstrap) fn bind_history<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
    history: HistoryRef,
) {
    let store = store(scope);
    let id = next_id(&store);
    let weak_store = Rc::downgrade(&store);
    let weak = v8::Weak::with_finalizer(
        scope,
        wrapper,
        Box::new(move |_| {
            if let Some(store) = weak_store.upgrade() {
                store.borrow_mut().histories.remove(&id);
            }
        }),
    );
    store.borrow_mut().histories.insert(id, (weak, history));
    set_private_value(
        scope,
        wrapper,
        HISTORY_ID,
        v8::BigInt::new_from_u64(scope, id).into(),
    );
}

pub(in crate::context_bootstrap) fn history<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
) -> Option<HistoryRef> {
    let id = id(scope, wrapper, HISTORY_ID)?;
    scope
        .get_slot::<Store>()?
        .borrow()
        .histories
        .get(&id)
        .map(|(_, record)| record.clone())
}

pub(in crate::context_bootstrap) fn bind_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
    entry: HistoryEntryRef,
) {
    let store = store(scope);
    let id = next_id(&store);
    let record_key = Rc::as_ptr(&entry) as usize;
    let weak_store = Rc::downgrade(&store);
    let weak = v8::Weak::with_finalizer(
        scope,
        wrapper,
        Box::new(move |_| {
            if let Some(store) = weak_store.upgrade() {
                let mut records = store.borrow_mut();
                records.entries.remove(&id);
                if let Some(ids) = records.entry_ids.get_mut(&record_key) {
                    ids.retain(|candidate| *candidate != id);
                    if ids.is_empty() {
                        records.entry_ids.remove(&record_key);
                    }
                }
            }
        }),
    );
    store
        .borrow_mut()
        .entries
        .insert(id, (Rc::new(weak), entry));
    store
        .borrow_mut()
        .entry_ids
        .entry(record_key)
        .or_default()
        .push(id);
    set_private_value(
        scope,
        wrapper,
        ENTRY_ID,
        v8::BigInt::new_from_u64(scope, id).into(),
    );
}

pub(in crate::context_bootstrap) fn entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
) -> Option<HistoryEntryRef> {
    let id = id(scope, wrapper, ENTRY_ID)?;
    scope
        .get_slot::<Store>()?
        .borrow()
        .entries
        .get(&id)
        .map(|(_, record)| record.clone())
}

/// V8 wrappers are world-local, while their record and event target are shared.
/// Weak handles preserve identity for observable wrappers without retaining a
/// destroyed isolated realm from the Window's canonical wrapper cache.
pub(in crate::context_bootstrap) fn entry_in_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Context>,
) -> v8::Local<'s, v8::Object> {
    if source.get_creation_context(scope) == Some(context) {
        return source;
    }
    let Some(record) = entry(scope, source) else {
        return source;
    };
    let registry = store(scope);
    let candidates: Vec<_> = {
        let records = registry.borrow();
        records
            .entry_ids
            .get(&(Rc::as_ptr(&record) as usize))
            .into_iter()
            .flatten()
            .filter_map(|id| records.entries.get(id))
            .map(|(wrapper, _)| wrapper.clone())
            .collect()
    };
    for candidate in candidates {
        if let Some(wrapper) = candidate.to_local(scope)
            && wrapper.get_creation_context(scope) == Some(context)
        {
            return wrapper;
        }
    }
    let owner = super::super::navigation_window::runtime_window_owner(scope, source);
    let target = super::super::shared_event_targets::shared_target_owner(scope, source);
    let scope = &mut v8::ContextScope::new(scope, context);
    let wrapper = super::super::navigation_entry::wrap_native_navigation_entry(scope, record);
    super::super::navigation_window::set_runtime_window_owner(scope, wrapper, owner);
    super::super::shared_event_targets::bind_shared_target(scope, wrapper, target);
    wrapper
}

pub(in crate::context_bootstrap) fn entry_value_in_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: v8::Local<'s, v8::Context>,
) -> v8::Local<'s, v8::Value> {
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return value;
    };
    if entry(scope, object).is_none() {
        return value;
    }
    entry_in_realm(scope, object, context).into()
}

fn wrappers<'s>(
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

pub(in crate::context_bootstrap) fn cache_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    wrapper: v8::Local<'s, v8::Object>,
) {
    let Some(entry) = entry(scope, wrapper) else {
        return;
    };
    let id = entry.borrow().id.clone();
    let Some(key) = v8_string(scope, &id) else {
        return;
    };
    let _ = wrappers(scope, owner).set(scope, key.into(), wrapper.into());
}

pub(in crate::context_bootstrap) fn entry_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    entry: HistoryEntryRef,
) -> v8::Local<'s, v8::Object> {
    let context = owner
        .get_creation_context(scope)
        .expect("History Window realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    let id = entry.borrow().id.clone();
    let map = wrappers(scope, owner);
    if let Some(key) = v8_string(scope, &id)
        && let Some(wrapper) = map
            .get(scope, key.into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        && self::entry(scope, wrapper).is_some_and(|record| Rc::ptr_eq(&record, &entry))
    {
        return wrapper;
    }
    let wrapper = super::super::navigation_entry::wrap_native_navigation_entry(scope, entry);
    super::super::navigation_activation::bind_navigation_entry_runtime_owner(scope, wrapper, owner);
    wrapper
}

pub(in crate::context_bootstrap) fn prune_entry_wrappers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    entries: &[HistoryEntryRef],
) {
    let live: std::collections::HashMap<_, _> = entries
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
            .and_then(|wrapper| entry(scope, wrapper));
        let retained = record
            .is_some_and(|record| live.get(&record.borrow().id) == Some(&Rc::as_ptr(&record)));
        if !retained {
            let _ = map.delete(scope, key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_history::{HistoryEntry, ScrollRestoration};
    use moli_session_history::{NavigationHistoryDocumentId, NavigationHistoryEntryKey};

    #[test]
    fn native_history_outlives_entry_wrappers_and_is_reclaimed_with_its_last_wrapper() {
        moli_v8_test_util::ensure_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        let (keep_alive, history_weak, entry_weak) = {
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let entry = HistoryEntry {
                url: "https://example.test/".to_owned(),
                id: "entry".to_owned(),
                key: NavigationHistoryEntryKey::allocate(),
                document: NavigationHistoryDocumentId::allocate(),
                index: 0,
                referrer_policy: None,
                history_state: None,
                navigation_state: None,
                scroll_restoration: ScrollRestoration::Auto,
                scroll_offset: None,
            }
            .into_ref();
            let history = Rc::new(RefCell::new(WindowHistory::new(vec![entry.clone()], 0)));
            let history_weak = Rc::downgrade(&history);
            let entry_weak = Rc::downgrade(&entry);
            let entry_wrapper = v8::Object::new(scope);
            bind_entry(scope, entry_wrapper, entry);
            let default_wrapper = v8::Object::new(scope);
            bind_history(scope, default_wrapper, history.clone());
            let isolated_wrapper = v8::Object::new(scope);
            bind_history(scope, isolated_wrapper, history);
            (
                v8::Global::new(scope, isolated_wrapper),
                history_weak,
                entry_weak,
            )
        };
        isolate.low_memory_notification();
        assert!(history_weak.upgrade().is_some());
        assert!(entry_weak.upgrade().is_some());
        assert!(
            isolate
                .get_slot::<Store>()
                .unwrap()
                .borrow()
                .entries
                .is_empty()
        );
        drop(keep_alive);
        isolate.low_memory_notification();
        assert!(history_weak.upgrade().is_none());
        assert!(entry_weak.upgrade().is_none());
        assert!(
            isolate
                .get_slot::<Store>()
                .unwrap()
                .borrow()
                .histories
                .is_empty()
        );
    }
}
