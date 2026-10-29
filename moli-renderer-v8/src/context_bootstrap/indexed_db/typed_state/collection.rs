use super::*;

pub(super) const OBJECT_STORE_TRANSACTION: &str = "moli.IndexedDb.traced.objectStoreTransaction";
pub(super) const OBJECT_STORE_DATABASE: &str = "moli.IndexedDb.traced.objectStoreDatabase";
pub(super) const INDEX_OBJECT_STORE: &str = "moli.IndexedDb.traced.indexObjectStore";

// A finalizer only releases its own native payload and queues an id. It must
// never borrow the runtime table: V8 can collect while another state lookup
// holds that table's RefCell guard.
pub(super) fn track_collectible_wrapper<T: 'static>(
    scope: &mut v8::PinScope<'_, '_>,
    wrapper: v8::Local<'_, v8::Object>,
    id: IndexedDbObjectId,
    table: &Rc<RefCell<IndexedDbRuntimeStateTable>>,
    payload: T,
) {
    let collected = Rc::clone(&table.borrow().collected_wrappers);
    let owner = v8::Weak::with_finalizer(
        scope,
        wrapper,
        Box::new(move |_| {
            drop(payload);
            collected.borrow_mut().push(id);
        }),
    );
    table.borrow_mut().collectible_wrappers.insert(id, owner);
}

impl IndexedDbRuntimeStateTable {
    pub(super) fn sweep_collected_wrappers(&mut self) {
        for id in self.collected_wrappers.borrow_mut().drain(..) {
            self.collectible_wrappers.remove(&id);
            self.wrappers.remove(&id);
            self.requests.remove(&id);
            self.cursors.remove(&id);
            self.object_stores.remove(&id);
            self.indexes.remove(&id);
        }
    }
}

pub(super) fn is_request_value_slot(key: &str) -> bool {
    matches!(
        key,
        INDEXED_DB_REQUEST_SOURCE_SLOT
            | INDEXED_DB_REQUEST_TRANSACTION_SLOT
            | INDEXED_DB_REQUEST_RESULT_SLOT
            | INDEXED_DB_REQUEST_ERROR_SLOT
            | INDEXED_DB_PENDING_RESULT_SLOT
            | INDEXED_DB_PENDING_ERROR_SLOT
            | INDEXED_DB_PENDING_CURSOR_SLOT
    )
}

pub(super) fn request_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'_, v8::Object>,
    slot: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let key = private_key(scope, slot)?;
    // An explicitly stored undefined is a valid request result. An absent
    // pending slot means there is no delivery waiting for this request.
    if request.has_private(scope, key) != Some(true) {
        return None;
    }
    request.get_private(scope, key)
}

pub(super) fn clear_pending_request_values(
    scope: &mut v8::PinScope<'_, '_>,
    request: v8::Local<'_, v8::Object>,
) {
    for slot in [
        INDEXED_DB_PENDING_RESULT_SLOT,
        INDEXED_DB_PENDING_ERROR_SLOT,
        INDEXED_DB_PENDING_CURSOR_SLOT,
    ] {
        if let Some(key) = private_key(scope, slot) {
            let _ = request.delete_private(scope, key);
        }
    }
}

pub(super) fn cursor_native_state(
    scope: &mut v8::PinScope<'_, '_>,
    cursor: v8::Local<'_, v8::Object>,
) -> Option<Rc<RefCell<IndexedDbCursorLifecycleState>>> {
    let id = indexed_db_typed_state_id(scope, cursor)?;
    let table = indexed_db_runtime_state_table_for_object(scope, cursor);
    table.borrow().cursors.get(&id)?.upgrade()
}
