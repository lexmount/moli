//! Realm wrappers lease native records. The registry holds only weak V8
//! handles, so a wrapper/Window cycle cannot keep a retired history alive.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use moli_history::{HistoryEntryRef, WindowHistory};

use crate::util::{get_private_value, set_private_value};

pub(in crate::context_bootstrap) type HistoryRef = Rc<RefCell<WindowHistory>>;
type Store = Rc<RefCell<NativeRecords>>;
const HISTORY_ID: &str = "__moliNativeHistory";
const ENTRY_ID: &str = "__moliNativeHistoryEntry";

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

/// Find an observable wrapper for this native record in the requested realm.
/// Clone the weak handles before touching V8 so finalizers can borrow the store.
pub(in crate::context_bootstrap) fn find_entry_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    record: &HistoryEntryRef,
    context: v8::Local<'s, v8::Context>,
) -> Option<v8::Local<'s, v8::Object>> {
    let registry = scope.get_slot::<Store>()?;
    let candidates: Vec<_> = {
        let records = registry.borrow();
        records
            .entry_ids
            .get(&(Rc::as_ptr(record) as usize))
            .into_iter()
            .flatten()
            .filter_map(|id| records.entries.get(id))
            .map(|(wrapper, _)| wrapper.clone())
            .collect()
    };
    for candidate in candidates {
        if let Some(wrapper) = candidate.to_local(scope)
            && super::super::world_wrappers::belongs_to_world(scope, wrapper, context)
        {
            return Some(wrapper);
        }
    }
    None
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
                inherited_origin: None,
                document_origin: "https://example.test".to_owned(),
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
