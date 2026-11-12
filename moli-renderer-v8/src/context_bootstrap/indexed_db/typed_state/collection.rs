use super::*;

pub(in crate::context_bootstrap::indexed_db) const OBJECT_STORE_TRANSACTION: &str =
    "moli.IndexedDb.traced.objectStoreTransaction";
pub(in crate::context_bootstrap::indexed_db) const OBJECT_STORE_DATABASE: &str =
    "moli.IndexedDb.traced.objectStoreDatabase";
pub(in crate::context_bootstrap::indexed_db) const INDEX_OBJECT_STORE: &str =
    "moli.IndexedDb.traced.indexObjectStore";

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
            self.transactions.remove(&id);
            self.databases.remove(&id);
            self.cursors.remove(&id);
            self.object_stores.remove(&id);
            self.indexes.remove(&id);
        }
    }
}

// These edges belong to the request in the V8 heap, so result/source cycles
// remain collectible. Scalar lifecycle state never passes through these keys.
#[derive(Clone, Copy)]
pub(in crate::context_bootstrap::indexed_db) enum IndexedDbRequestValue {
    Source,
    Transaction,
    Result,
    Error,
    PendingResult,
    PendingError,
    PendingCursor,
}

impl IndexedDbRequestValue {
    fn key(self) -> &'static str {
        match self {
            Self::Source => "moli.IndexedDb.requestSource",
            Self::Transaction => "moli.IndexedDb.requestTransaction",
            Self::Result => "moli.IndexedDb.requestResult",
            Self::Error => "moli.IndexedDb.requestError",
            Self::PendingResult => "moli.IndexedDb.pendingResult",
            Self::PendingError => "moli.IndexedDb.pendingError",
            Self::PendingCursor => "moli.IndexedDb.pendingCursor",
        }
    }
}

pub(in crate::context_bootstrap::indexed_db) fn request_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'_, v8::Object>,
    field: IndexedDbRequestValue,
) -> Option<v8::Local<'s, v8::Value>> {
    let id = indexed_db_typed_state_id(scope, request)?;
    let table = indexed_db_runtime_state_table_for_object(scope, request);
    if !table.borrow().requests.contains_key(&id) {
        return None;
    }
    let key = private_key(scope, field.key())?;
    // An explicitly stored undefined is a valid request result. An absent
    // pending value means there is no delivery waiting for this request.
    if request.has_private(scope, key) != Some(true) {
        return None;
    }
    request.get_private(scope, key)
}

pub(in crate::context_bootstrap::indexed_db) fn set_request_value(
    scope: &mut v8::PinScope<'_, '_>,
    request: v8::Local<'_, v8::Object>,
    field: IndexedDbRequestValue,
    value: v8::Local<'_, v8::Value>,
) {
    let Some(id) = indexed_db_typed_state_id(scope, request) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, request);
    let registered = table.borrow().requests.contains_key(&id);
    if registered {
        set_private_value(scope, request, field.key(), value);
    }
}

pub(super) fn clear_pending_request_values(
    scope: &mut v8::PinScope<'_, '_>,
    request: v8::Local<'_, v8::Object>,
) {
    for slot in [
        IndexedDbRequestValue::PendingResult,
        IndexedDbRequestValue::PendingError,
        IndexedDbRequestValue::PendingCursor,
    ] {
        if let Some(key) = private_key(scope, slot.key()) {
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

pub(super) fn object_store_native_state(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> Option<Rc<RefCell<IndexedDbObjectStoreLifecycleState>>> {
    let id = indexed_db_typed_state_id(scope, object)?;
    let table = indexed_db_runtime_state_table_for_object(scope, object);
    table.borrow().object_stores.get(&id)?.upgrade()
}
pub(super) fn index_native_state(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> Option<Rc<RefCell<IndexedDbIndexLifecycleState>>> {
    let id = indexed_db_typed_state_id(scope, object)?;
    let table = indexed_db_runtime_state_table_for_object(scope, object);
    table.borrow().indexes.get(&id)?.upgrade()
}
