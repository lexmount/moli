//! A live Storage wrapper pins only its own area. Native dirty/in-flight
//! batches keep the same area alive independently of the wrapper's lifetime.
use crate::util::{get_private_value, set_private_value};
use moli_web_storage::{SharedWebStorageStore, WebStorageAreaLease};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

const LEASE_SLOT: &str = "__moliWebStorageAreaLease";
#[derive(Default)]
struct Leases {
    next: u64,
    entries: HashMap<u64, Entry>,
}
struct Entry {
    store: SharedWebStorageStore,
    key: String,
    _lease: WebStorageAreaLease,
}
type Registry = Rc<RefCell<Leases>>;

pub(super) fn ensure_storage_area_lease<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    storage: v8::Local<'s, v8::Object>,
    store: &SharedWebStorageStore,
    key: &str,
) -> Option<()> {
    let registry = scope.get_slot::<Registry>().cloned().unwrap_or_else(|| {
        let registry = Registry::default();
        scope.set_slot(registry.clone());
        registry
    });
    let id = get_private_value(scope, storage, LEASE_SLOT)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .map(|id| id.u64_value().0);
    if let Some(id) = id
        && registry
            .borrow()
            .entries
            .get(&id)
            .is_some_and(|entry| std::sync::Arc::ptr_eq(&entry.store, store) && entry.key == key)
    {
        return Some(());
    }
    let lease = match store.lease_area(key) {
        Ok(Some(lease)) => lease,
        Ok(None) => return Some(()),
        Err(error) => {
            super::helpers::throw_storage_persistence_error(scope, &error.to_string());
            return None;
        }
    };
    if let Some(id) = id {
        let old = registry.borrow_mut().entries.insert(
            id,
            Entry {
                store: store.clone(),
                key: key.to_owned(),
                _lease: lease,
            },
        );
        drop(old);
        return Some(());
    }
    let id = {
        let mut registry = registry.borrow_mut();
        registry.next = registry.next.checked_add(1)?;
        let id = registry.next;
        registry.entries.insert(
            id,
            Entry {
                store: store.clone(),
                key: key.to_owned(),
                _lease: lease,
            },
        );
        id
    };
    let marker = v8::BigInt::new_from_u64(scope, id);
    set_private_value(scope, storage, LEASE_SLOT, marker.into());
    let weak = Rc::downgrade(&registry);
    crate::v8_finalizer::track_context_owned_v8_finalizer(scope, storage, move || {
        if let Some(registry) = weak.upgrade() {
            let retired = registry.borrow_mut().entries.remove(&id);
            drop(retired);
        }
    });
    Some(())
}
