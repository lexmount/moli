//! IndexedDB manager access bound to the current V8 context.

use super::{IndexedDbError, IndexedDbManager};
use crate::context_bootstrap::indexed_db::WeakIndexedDbManager;

#[derive(Clone, Debug)]
pub(crate) struct IndexedDbManagerSlot(pub(crate) Option<WeakIndexedDbManager>);

pub(crate) fn set_indexed_db_manager_for_context(
    context: v8::Local<'_, v8::Context>,
    manager: Option<WeakIndexedDbManager>,
) {
    let _previous = context.set_slot(std::rc::Rc::new(IndexedDbManagerSlot(manager)));
}

pub(in crate::context_bootstrap::indexed_db) fn with_indexed_db_manager<R>(
    scope: &mut v8::PinScope<'_, '_>,
    f: impl FnOnce(&mut IndexedDbManager) -> std::result::Result<R, IndexedDbError>,
) -> std::result::Result<R, IndexedDbError> {
    let manager = scope
        .get_current_context()
        .get_slot::<IndexedDbManagerSlot>()
        .as_deref()
        .and_then(|slot| slot.0.as_ref())
        .and_then(WeakIndexedDbManager::upgrade)
        .ok_or_else(|| {
            IndexedDbError::InvalidState("IndexedDB browser context is closed".to_owned())
        })?;
    let mut manager = manager.lock();
    f(&mut manager)
}

pub(in crate::context_bootstrap) fn indexed_db_usage_bytes_for_storage_key(
    scope: &mut v8::PinScope<'_, '_>,
    storage_key: &str,
) -> u64 {
    let usage = with_indexed_db_manager(scope, |manager| manager.origin_usage_bytes(storage_key));
    usage.unwrap_or(0)
}

#[cfg(test)]
pub(crate) fn indexed_db_manager_context_slot_present_for_test(
    scope: &mut v8::PinScope<'_, '_>,
) -> bool {
    scope
        .get_current_context()
        .get_slot::<IndexedDbManagerSlot>()
        .is_some()
}

#[cfg(test)]
pub(crate) fn indexed_db_manager_isolate_slot_present_for_test(
    scope: &mut v8::PinScope<'_, '_>,
) -> bool {
    scope.get_slot::<IndexedDbManagerSlot>().is_some()
}
