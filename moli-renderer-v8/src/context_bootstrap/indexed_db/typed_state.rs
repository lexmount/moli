use super::*;
use moli_storage_service::StorageBucketIdentity;
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct IndexedDbObjectId(u64);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct IndexedDbTaskId(u64);

impl IndexedDbTaskId {
    #[cfg(test)]
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum IndexedDbWrapperKind {
    Factory,
    Database,
    Request,
    OpenRequest,
    Transaction,
    Cursor,
    ObjectStore,
    Index,
    KeyRange,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum IndexedDbTaskKind {
    RequestSuccess,
    RequestError,
    Open,
    OpenBlocked,
    DeleteBlocked,
    DrainBlockedOpens,
    DatabasesSettle,
    TransactionStart,
    TransactionCommit,
    TransactionAbort,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum IndexedDbExecutionOwner {
    PendingWindow(crate::native_bridge::OwnerDispatchScope),
    Window(crate::native_bridge::WindowExecutionContextIdentity),
}

impl IndexedDbExecutionOwner {
    fn dispatch_scope(self) -> crate::native_bridge::OwnerDispatchScope {
        match self {
            Self::PendingWindow(dispatch_scope) => dispatch_scope,
            Self::Window(execution_context) => execution_context.dispatch_scope(),
        }
    }

    pub(super) fn execution_context(
        self,
    ) -> Option<crate::native_bridge::WindowExecutionContextIdentity> {
        match self {
            Self::PendingWindow(_) => None,
            Self::Window(execution_context) => Some(execution_context),
        }
    }

    pub(super) fn for_window_execution_context(
        execution_context: crate::native_bridge::WindowExecutionContextIdentity,
    ) -> Self {
        Self::Window(execution_context)
    }

    #[cfg(test)]
    fn without_execution_context(dispatch_scope: crate::native_bridge::OwnerDispatchScope) -> Self {
        Self::PendingWindow(dispatch_scope)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct IndexedDbStorageScope {
    storage_key: String,
    bucket_identity: Option<StorageBucketIdentity>,
    browser_context_id: String,
    profile_partition_id: String,
}

impl IndexedDbStorageScope {
    pub(super) fn new(
        storage_key: impl Into<String>,
        browser_context_id: impl Into<String>,
        profile_partition_id: impl Into<String>,
    ) -> Self {
        Self {
            storage_key: storage_key.into(),
            bucket_identity: None,
            browser_context_id: browser_context_id.into(),
            profile_partition_id: profile_partition_id.into(),
        }
    }

    pub(super) fn with_bucket_identity(mut self, identity: StorageBucketIdentity) -> Self {
        self.bucket_identity = Some(identity);
        self
    }

    pub(super) fn storage_key(&self) -> &str {
        &self.storage_key
    }

    pub(super) fn bucket_identity(&self) -> Option<&StorageBucketIdentity> {
        self.bucket_identity.as_ref()
    }

    pub(super) fn browser_context_id(&self) -> &str {
        &self.browser_context_id
    }

    pub(super) fn profile_partition_id(&self) -> &str {
        &self.profile_partition_id
    }

    fn has_explicit_partition_boundary(&self) -> bool {
        !self.browser_context_id().is_empty() && !self.profile_partition_id().is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct IndexedDbWrapperState {
    kind: IndexedDbWrapperKind,
    owner: IndexedDbExecutionOwner,
    storage_scope: Option<IndexedDbStorageScope>,
}

impl IndexedDbWrapperState {
    fn new(
        kind: IndexedDbWrapperKind,
        owner: IndexedDbExecutionOwner,
        storage_scope: Option<IndexedDbStorageScope>,
    ) -> Self {
        Self {
            kind,
            owner,
            storage_scope,
        }
    }

    fn owner_for_dispatch(&self) -> crate::native_bridge::OwnerDispatchScope {
        match self.kind {
            IndexedDbWrapperKind::Factory
            | IndexedDbWrapperKind::Database
            | IndexedDbWrapperKind::Request
            | IndexedDbWrapperKind::OpenRequest
            | IndexedDbWrapperKind::Transaction
            | IndexedDbWrapperKind::Cursor
            | IndexedDbWrapperKind::ObjectStore
            | IndexedDbWrapperKind::Index
            | IndexedDbWrapperKind::KeyRange => self.owner.dispatch_scope(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct IndexedDbTaskState {
    kind: IndexedDbTaskKind,
    owner: IndexedDbExecutionOwner,
    storage_scope: Option<IndexedDbStorageScope>,
}

impl IndexedDbTaskState {
    fn new(
        kind: IndexedDbTaskKind,
        owner: IndexedDbExecutionOwner,
        storage_scope: Option<IndexedDbStorageScope>,
    ) -> Self {
        Self {
            kind,
            owner,
            storage_scope,
        }
    }

    fn owner_for_dispatch(&self) -> crate::native_bridge::OwnerDispatchScope {
        match self.kind {
            IndexedDbTaskKind::RequestSuccess
            | IndexedDbTaskKind::RequestError
            | IndexedDbTaskKind::Open
            | IndexedDbTaskKind::OpenBlocked
            | IndexedDbTaskKind::DeleteBlocked
            | IndexedDbTaskKind::DrainBlockedOpens
            | IndexedDbTaskKind::DatabasesSettle
            | IndexedDbTaskKind::TransactionStart
            | IndexedDbTaskKind::TransactionCommit
            | IndexedDbTaskKind::TransactionAbort => self.owner.dispatch_scope(),
        }
    }
}

struct IndexedDbDatabasesSettleTaskPayload {
    resolver: v8::Global<v8::Value>,
    value: v8::Global<v8::Value>,
    reject: bool,
}

impl IndexedDbDatabasesSettleTaskPayload {
    fn new(
        scope: &mut v8::PinScope<'_, '_>,
        resolver: v8::Local<'_, v8::PromiseResolver>,
        value: v8::Local<'_, v8::Value>,
        reject: bool,
    ) -> Self {
        let resolver: v8::Local<'_, v8::Value> = resolver.into();
        Self {
            resolver: v8::Global::new(scope, resolver),
            value: v8::Global::new(scope, value),
            reject,
        }
    }
}

struct IndexedDbRequestDispatchTaskPayload {
    request: v8::Global<v8::Value>,
}

impl IndexedDbRequestDispatchTaskPayload {
    fn new(scope: &mut v8::PinScope<'_, '_>, request: v8::Local<'_, v8::Object>) -> Self {
        let request: v8::Local<'_, v8::Value> = request.into();
        Self {
            request: v8::Global::new(scope, request),
        }
    }
}

struct IndexedDbOpenTaskPayload {
    request: v8::Global<v8::Value>,
    database: v8::Global<v8::Value>,
    transaction: v8::Global<v8::Value>,
    old_version: u64,
    new_version: u64,
}

impl IndexedDbOpenTaskPayload {
    fn new(
        scope: &mut v8::PinScope<'_, '_>,
        request: v8::Local<'_, v8::Object>,
        database: v8::Local<'_, v8::Object>,
        transaction: v8::Local<'_, v8::Object>,
        old_version: u64,
        new_version: u64,
    ) -> Self {
        let request: v8::Local<'_, v8::Value> = request.into();
        let database: v8::Local<'_, v8::Value> = database.into();
        let transaction: v8::Local<'_, v8::Value> = transaction.into();
        Self {
            request: v8::Global::new(scope, request),
            database: v8::Global::new(scope, database),
            transaction: v8::Global::new(scope, transaction),
            old_version,
            new_version,
        }
    }
}

struct IndexedDbBlockedTaskPayload {
    request: v8::Global<v8::Value>,
    origin: String,
    name: String,
    version: Option<u64>,
    old_version: u64,
    new_version: Option<u64>,
}

impl IndexedDbBlockedTaskPayload {
    fn new(
        scope: &mut v8::PinScope<'_, '_>,
        request: v8::Local<'_, v8::Object>,
        origin: impl Into<String>,
        name: impl Into<String>,
        version: Option<u64>,
        old_version: u64,
        new_version: Option<u64>,
    ) -> Self {
        let request: v8::Local<'_, v8::Value> = request.into();
        Self {
            request: v8::Global::new(scope, request),
            origin: origin.into(),
            name: name.into(),
            version,
            old_version,
            new_version,
        }
    }
}

pub(super) struct IndexedDbBlockedTaskPayloadLocals<'s> {
    pub(super) request: v8::Local<'s, v8::Object>,
    pub(super) origin: String,
    pub(super) name: String,
    pub(super) version: Option<u64>,
    pub(super) old_version: u64,
    pub(super) new_version: Option<u64>,
}

struct IndexedDbTransactionTaskPayload {
    transaction: v8::Global<v8::Value>,
}

impl IndexedDbTransactionTaskPayload {
    fn new(scope: &mut v8::PinScope<'_, '_>, transaction: v8::Local<'_, v8::Object>) -> Self {
        let transaction: v8::Local<'_, v8::Value> = transaction.into();
        Self {
            transaction: v8::Global::new(scope, transaction),
        }
    }
}

struct IndexedDbRequestLifecycleState {
    source: v8::Global<v8::Value>,
    transaction: v8::Global<v8::Value>,
    ready_state: String,
    result: v8::Global<v8::Value>,
    error: v8::Global<v8::Value>,
    blocked_dispatched: bool,
    pending_result: Option<v8::Global<v8::Value>>,
    pending_error: Option<v8::Global<v8::Value>>,
    pending_cursor: Option<v8::Global<v8::Value>>,
    pending_cursor_position: Option<f64>,
}

impl IndexedDbRequestLifecycleState {
    fn new(
        scope: &mut v8::PinScope<'_, '_>,
        source: v8::Local<'_, v8::Value>,
        transaction: v8::Local<'_, v8::Value>,
        blocked_dispatched: bool,
    ) -> Self {
        let result: v8::Local<'_, v8::Value> = v8::undefined(scope).into();
        let error: v8::Local<'_, v8::Value> = v8::null(scope).into();
        Self {
            source: v8::Global::new(scope, source),
            transaction: v8::Global::new(scope, transaction),
            ready_state: "pending".to_owned(),
            result: v8::Global::new(scope, result),
            error: v8::Global::new(scope, error),
            blocked_dispatched,
            pending_result: None,
            pending_error: None,
            pending_cursor: None,
            pending_cursor_position: None,
        }
    }
}

struct IndexedDbTransactionLifecycleState {
    handle: Option<TransactionHandle>,
    active: bool,
    finished: bool,
    aborted: bool,
    started: bool,
    start_scheduled: bool,
    abort_dispatched: bool,
    pending: u32,
    commit_scheduled: bool,
    deactivation_scheduled: bool,
    operations_waiting_for_start: Vec<IndexedDbPendingTransactionOperation>,
    db_key: Option<String>,
}

impl IndexedDbTransactionLifecycleState {
    fn new(handle: Option<TransactionHandle>, started: bool, db_key: Option<String>) -> Self {
        Self {
            handle,
            active: true,
            finished: false,
            aborted: false,
            started,
            start_scheduled: false,
            abort_dispatched: false,
            pending: 0,
            commit_scheduled: false,
            deactivation_scheduled: false,
            operations_waiting_for_start: Vec::new(),
            db_key,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct IndexedDbObjectStoreMetadata {
    info: ObjectStoreInfo,
    indexes: BTreeMap<String, IndexInfo>,
}

impl IndexedDbObjectStoreMetadata {
    pub(super) fn new(info: ObjectStoreInfo, indexes: impl IntoIterator<Item = IndexInfo>) -> Self {
        let indexes = indexes
            .into_iter()
            .map(|index| (index.name.clone(), index))
            .collect();
        Self { info, indexes }
    }

    pub(super) fn info(&self) -> &ObjectStoreInfo {
        &self.info
    }

    pub(super) fn index(&self, name: &str) -> Option<&IndexInfo> {
        self.indexes.get(name)
    }

    pub(super) fn indexes_in_name_order(&self) -> Vec<IndexInfo> {
        self.info
            .index_names
            .iter()
            .filter_map(|name| self.indexes.get(name).cloned())
            .collect()
    }

    fn set_index(&mut self, info: IndexInfo) {
        if !self.info.index_names.iter().any(|name| name == &info.name) {
            self.info.index_names.push(info.name.clone());
        }
        self.indexes.insert(info.name.clone(), info);
    }

    fn remove_index(&mut self, name: &str) {
        self.info.index_names.retain(|candidate| candidate != name);
        self.indexes.remove(name);
    }
}

struct IndexedDbDatabaseLifecycleState {
    handle: DatabaseHandle,
    database_key: String,
    storage_scope: IndexedDbStorageScope,
    closed: bool,
    metadata: BTreeMap<String, IndexedDbObjectStoreMetadata>,
    upgrade_transaction: Option<v8::Global<v8::Value>>,
}

impl IndexedDbDatabaseLifecycleState {
    fn new(
        handle: DatabaseHandle,
        database_key: String,
        storage_scope: IndexedDbStorageScope,
    ) -> Self {
        Self {
            handle,
            database_key,
            storage_scope,
            closed: false,
            metadata: BTreeMap::new(),
            upgrade_transaction: None,
        }
    }
}

struct IndexedDbCursorLifecycleState {
    request: v8::Global<v8::Value>,
    entries: v8::Global<v8::Value>,
    key_only: bool,
    position: f64,
}

impl IndexedDbCursorLifecycleState {
    fn new(
        scope: &mut v8::PinScope<'_, '_>,
        request: v8::Local<'_, v8::Object>,
        entries: v8::Local<'_, v8::Array>,
        key_only: bool,
        position: f64,
    ) -> Self {
        let request: v8::Local<'_, v8::Value> = request.into();
        let entries: v8::Local<'_, v8::Value> = entries.into();
        Self {
            request: v8::Global::new(scope, request),
            entries: v8::Global::new(scope, entries),
            key_only,
            position,
        }
    }
}

struct IndexedDbObjectStoreLifecycleState {
    transaction: v8::Global<v8::Value>,
    database: v8::Global<v8::Value>,
    name: String,
    metadata: IndexedDbObjectStoreMetadata,
}

impl IndexedDbObjectStoreLifecycleState {
    fn new(
        scope: &mut v8::PinScope<'_, '_>,
        transaction: v8::Local<'_, v8::Object>,
        database: v8::Local<'_, v8::Object>,
        metadata: IndexedDbObjectStoreMetadata,
    ) -> Self {
        let name = metadata.info.name.clone();
        Self {
            transaction: v8::Global::new(scope, v8::Local::<v8::Value>::from(transaction)),
            database: v8::Global::new(scope, v8::Local::<v8::Value>::from(database)),
            name,
            metadata,
        }
    }
}

struct IndexedDbIndexLifecycleState {
    object_store: v8::Global<v8::Value>,
    info: IndexInfo,
    marker: bool,
}

impl IndexedDbIndexLifecycleState {
    fn new(
        scope: &mut v8::PinScope<'_, '_>,
        object_store: v8::Local<'_, v8::Object>,
        info: IndexInfo,
    ) -> Self {
        Self {
            object_store: v8::Global::new(scope, v8::Local::<v8::Value>::from(object_store)),
            info,
            marker: true,
        }
    }
}

struct IndexedDbKeyRangeLifecycleState {
    marker: bool,
}

#[derive(Default)]
pub(super) struct IndexedDbRuntimeStateTable {
    next_id: u64,
    wrappers: BTreeMap<IndexedDbObjectId, IndexedDbWrapperState>,
    tasks: BTreeMap<IndexedDbTaskId, IndexedDbTaskState>,
    databases_settle_tasks: BTreeMap<IndexedDbTaskId, IndexedDbDatabasesSettleTaskPayload>,
    request_dispatch_tasks: BTreeMap<IndexedDbTaskId, IndexedDbRequestDispatchTaskPayload>,
    open_tasks: BTreeMap<IndexedDbTaskId, IndexedDbOpenTaskPayload>,
    blocked_tasks: BTreeMap<IndexedDbTaskId, IndexedDbBlockedTaskPayload>,
    transaction_tasks: BTreeMap<IndexedDbTaskId, IndexedDbTransactionTaskPayload>,
    requests: BTreeMap<IndexedDbObjectId, IndexedDbRequestLifecycleState>,
    transactions: BTreeMap<IndexedDbObjectId, IndexedDbTransactionLifecycleState>,
    databases: BTreeMap<IndexedDbObjectId, IndexedDbDatabaseLifecycleState>,
    cursors: BTreeMap<IndexedDbObjectId, IndexedDbCursorLifecycleState>,
    object_stores: BTreeMap<IndexedDbObjectId, IndexedDbObjectStoreLifecycleState>,
    indexes: BTreeMap<IndexedDbObjectId, IndexedDbIndexLifecycleState>,
    key_ranges: BTreeMap<IndexedDbObjectId, IndexedDbKeyRangeLifecycleState>,
}

impl IndexedDbRuntimeStateTable {
    fn upsert_wrapper(
        &mut self,
        id: Option<IndexedDbObjectId>,
        state: IndexedDbWrapperState,
    ) -> IndexedDbObjectId {
        let id = id.unwrap_or_else(|| {
            self.next_id = self.next_id.saturating_add(1).max(1);
            IndexedDbObjectId(self.next_id)
        });
        self.wrappers.insert(id, state);
        id
    }

    fn wrapper(&self, id: IndexedDbObjectId) -> Option<&IndexedDbWrapperState> {
        self.wrappers.get(&id)
    }

    fn upsert_task(
        &mut self,
        id: Option<IndexedDbTaskId>,
        state: IndexedDbTaskState,
    ) -> IndexedDbTaskId {
        let id = id.unwrap_or_else(|| {
            self.next_id = self.next_id.saturating_add(1).max(1);
            IndexedDbTaskId(self.next_id)
        });
        self.tasks.insert(id, state);
        id
    }

    fn task(&self, id: IndexedDbTaskId) -> Option<&IndexedDbTaskState> {
        self.tasks.get(&id)
    }
}

pub(super) fn ensure_indexed_db_runtime_state_table(
    scope: &mut v8::PinScope<'_, '_>,
) -> Rc<RefCell<IndexedDbRuntimeStateTable>> {
    let context = scope.get_current_context();
    ensure_indexed_db_runtime_state_table_for_context(context)
}

fn ensure_indexed_db_runtime_state_table_for_context(
    context: v8::Local<'_, v8::Context>,
) -> Rc<RefCell<IndexedDbRuntimeStateTable>> {
    if let Some(table) = context.get_slot::<RefCell<IndexedDbRuntimeStateTable>>() {
        return table;
    }
    let table = Rc::new(RefCell::new(IndexedDbRuntimeStateTable::default()));
    let _ = context.set_slot(table.clone());
    table
}

fn indexed_db_runtime_state_table_for_object(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> Rc<RefCell<IndexedDbRuntimeStateTable>> {
    let context = object
        .get_creation_context(scope)
        .expect("registered IndexedDB native objects retain their creation context");
    ensure_indexed_db_runtime_state_table_for_context(context)
}

pub(super) fn register_indexed_db_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
    kind: IndexedDbWrapperKind,
    storage_scope: Option<IndexedDbStorageScope>,
) {
    let owner = indexed_db_execution_owner_for_object(scope, wrapper);
    register_indexed_db_wrapper_with_owner(scope, wrapper, kind, owner, storage_scope);
}

pub(super) fn register_indexed_db_wrapper_with_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
    kind: IndexedDbWrapperKind,
    owner: IndexedDbExecutionOwner,
    storage_scope: Option<IndexedDbStorageScope>,
) {
    if let Some(storage_scope) = storage_scope.as_ref() {
        debug_assert!(storage_scope.has_explicit_partition_boundary());
    }
    let existing_id = indexed_db_typed_state_id(scope, wrapper);
    let table = indexed_db_runtime_state_table_for_object(scope, wrapper);
    let id = table.borrow_mut().upsert_wrapper(
        existing_id,
        IndexedDbWrapperState::new(kind, owner, storage_scope),
    );
    set_indexed_db_typed_state_id(scope, wrapper, id);
}

pub(super) fn register_indexed_db_request_lifecycle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    source: v8::Local<'s, v8::Value>,
    transaction: v8::Local<'s, v8::Value>,
    blocked_dispatched: bool,
) {
    let Some(id) = indexed_db_typed_state_id(scope, request) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, request);
    table.borrow_mut().requests.insert(
        id,
        IndexedDbRequestLifecycleState::new(scope, source, transaction, blocked_dispatched),
    );
}

pub(super) fn release_indexed_db_request_dispatch_refs<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) {
    let Some(id) = indexed_db_typed_state_id(scope, request) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, request);
    let mut table = table.borrow_mut();
    let Some(request) = table.requests.get_mut(&id) else {
        return;
    };
    request.pending_result = None;
    request.pending_error = None;
    request.pending_cursor = None;
    request.pending_cursor_position = None;
}

pub(super) fn register_indexed_db_transaction_lifecycle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    handle: Option<TransactionHandle>,
    started: bool,
    db_key: Option<String>,
) {
    let Some(id) = indexed_db_typed_state_id(scope, transaction) else {
        return;
    };
    let state = IndexedDbTransactionLifecycleState::new(handle, started, db_key);
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    table.borrow_mut().transactions.insert(id, state);
}

pub(in crate::context_bootstrap::indexed_db) fn schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) {
    let Some(id) = indexed_db_typed_state_id(scope, transaction) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    {
        let mut table = table.borrow_mut();
        let Some(state) = table.transactions.get_mut(&id) else {
            return;
        };
        if state.finished || state.deactivation_scheduled {
            return;
        }
        state.deactivation_scheduled = true;
    }
    crate::context_bootstrap::microtask_checkpoint::enqueue_indexed_db_transaction_deactivation(
        scope,
        transaction,
    );
}

pub(crate) fn deactivate_indexed_db_transaction_after_microtask_checkpoint<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) {
    let Some(id) = indexed_db_typed_state_id(scope, transaction) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    let should_commit = {
        let mut table = table.borrow_mut();
        let Some(state) = table.transactions.get_mut(&id) else {
            return;
        };
        state.deactivation_scheduled = false;
        if state.finished {
            return;
        }
        state.active = false;
        state.pending == 0
    };
    if should_commit {
        enqueue_transaction_commit_task(scope, transaction);
    }
}

pub(super) fn release_indexed_db_transaction_dispatch_refs<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) {
    let Some(id) = indexed_db_typed_state_id(scope, transaction) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    let mut table = table.borrow_mut();
    let Some(transaction) = table.transactions.get_mut(&id) else {
        return;
    };
    transaction.operations_waiting_for_start.clear();
}

pub(super) fn push_indexed_db_operation_waiting_for_start<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    operation: IndexedDbPendingTransactionOperation,
) -> Option<()> {
    let id = indexed_db_typed_state_id(scope, transaction)?;
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    table
        .borrow_mut()
        .transactions
        .get_mut(&id)?
        .operations_waiting_for_start
        .push(operation);
    Some(())
}

pub(super) fn take_indexed_db_operations_waiting_for_start<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) -> Vec<IndexedDbPendingTransactionOperation> {
    let Some(id) = indexed_db_typed_state_id(scope, transaction) else {
        return Vec::new();
    };
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    {
        let mut table = table.borrow_mut();
        let Some(transaction) = table.transactions.get_mut(&id) else {
            return Vec::new();
        };
        std::mem::take(&mut transaction.operations_waiting_for_start)
    }
}

pub(super) fn register_indexed_db_database_lifecycle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    handle: DatabaseHandle,
    database_key: String,
    storage_scope: IndexedDbStorageScope,
) {
    debug_assert!(storage_scope.has_explicit_partition_boundary());
    let Some(id) = indexed_db_typed_state_id(scope, database) else {
        return;
    };
    let state = IndexedDbDatabaseLifecycleState::new(handle, database_key, storage_scope);
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    table.borrow_mut().databases.insert(id, state);
}

pub(super) fn register_indexed_db_cursor_lifecycle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
    entries: v8::Local<'s, v8::Array>,
    key_only: bool,
    position: f64,
) {
    let Some(id) = indexed_db_typed_state_id(scope, cursor) else {
        return;
    };
    let state = IndexedDbCursorLifecycleState::new(scope, request, entries, key_only, position);
    let table = indexed_db_runtime_state_table_for_object(scope, cursor);
    table.borrow_mut().cursors.insert(id, state);
}

pub(super) fn register_indexed_db_object_store_lifecycle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
    transaction: v8::Local<'s, v8::Object>,
    database: v8::Local<'s, v8::Object>,
    metadata: IndexedDbObjectStoreMetadata,
) {
    let Some(id) = indexed_db_typed_state_id(scope, store) else {
        return;
    };
    let state = IndexedDbObjectStoreLifecycleState::new(scope, transaction, database, metadata);
    let table = indexed_db_runtime_state_table_for_object(scope, store);
    table.borrow_mut().object_stores.insert(id, state);
}

pub(super) fn register_indexed_db_index_lifecycle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: v8::Local<'s, v8::Object>,
    object_store: v8::Local<'s, v8::Object>,
    info: IndexInfo,
) {
    let Some(id) = indexed_db_typed_state_id(scope, index) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, index);
    table.borrow_mut().indexes.insert(
        id,
        IndexedDbIndexLifecycleState::new(scope, object_store, info),
    );
}

pub(super) fn register_indexed_db_key_range_lifecycle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key_range: v8::Local<'s, v8::Object>,
    marker: bool,
) {
    let Some(id) = indexed_db_typed_state_id(scope, key_range) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, key_range);
    table
        .borrow_mut()
        .key_ranges
        .insert(id, IndexedDbKeyRangeLifecycleState { marker });
}

pub(super) fn indexed_db_typed_owner_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
) -> Option<crate::native_bridge::OwnerDispatchScope> {
    indexed_db_typed_wrapper_state(scope, wrapper).map(|state| state.owner_for_dispatch())
}

pub(super) fn indexed_db_typed_execution_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
) -> Option<IndexedDbExecutionOwner> {
    indexed_db_typed_wrapper_state(scope, wrapper).map(|state| state.owner)
}

pub(super) fn indexed_db_typed_storage_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
) -> Option<IndexedDbStorageScope> {
    indexed_db_typed_wrapper_state(scope, wrapper).and_then(|state| state.storage_scope)
}

pub(super) fn indexed_db_typed_wrapper_is<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
    kind: IndexedDbWrapperKind,
) -> bool {
    indexed_db_typed_wrapper_state(scope, wrapper)
        .map(|state| state.kind == kind)
        .unwrap_or(false)
}

pub(super) fn register_indexed_db_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
    kind: IndexedDbTaskKind,
    storage_scope: Option<IndexedDbStorageScope>,
) {
    let owner = indexed_db_execution_owner_for_object(scope, task);
    register_indexed_db_task_with_owner(scope, task, kind, owner, storage_scope);
}

pub(super) fn indexed_db_typed_task_owner_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) -> Option<crate::native_bridge::OwnerDispatchScope> {
    indexed_db_typed_task_state(scope, task).map(|state| state.owner_for_dispatch())
}

pub(super) fn indexed_db_typed_task_execution_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) -> Option<crate::native_bridge::WindowExecutionContextIdentity> {
    indexed_db_typed_task_state(scope, task)?
        .owner
        .execution_context()
}

pub(super) fn indexed_db_typed_task_execution_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) -> Option<IndexedDbExecutionOwner> {
    indexed_db_typed_task_state(scope, task).map(|state| state.owner)
}

pub(super) fn indexed_db_typed_task_kind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) -> Option<IndexedDbTaskKind> {
    indexed_db_typed_task_state(scope, task).map(|state| state.kind)
}

pub(super) fn register_indexed_db_databases_settle_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
    owner: IndexedDbExecutionOwner,
    storage_scope: Option<IndexedDbStorageScope>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    value: v8::Local<'s, v8::Value>,
    reject: bool,
) {
    let id = register_indexed_db_task_with_owner(
        scope,
        task,
        IndexedDbTaskKind::DatabasesSettle,
        owner,
        storage_scope,
    );
    let payload = IndexedDbDatabasesSettleTaskPayload::new(scope, resolver, value, reject);
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    table
        .borrow_mut()
        .databases_settle_tasks
        .insert(id, payload);
}

pub(super) fn indexed_db_databases_settle_task_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) -> Option<(
    v8::Local<'s, v8::PromiseResolver>,
    v8::Local<'s, v8::Value>,
    bool,
)> {
    let id = indexed_db_typed_task_id(scope, task)?;
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    let table = table.borrow();
    let payload = table.databases_settle_tasks.get(&id)?;
    let resolver = v8::Local::new(scope, &payload.resolver);
    let resolver = v8::Local::<v8::Object>::try_from(resolver).ok()?;
    let resolver = unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(resolver) };
    let value = v8::Local::new(scope, &payload.value);
    Some((resolver, value, payload.reject))
}

pub(super) fn register_indexed_db_request_dispatch_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
    kind: IndexedDbTaskKind,
    request: v8::Local<'s, v8::Object>,
) {
    debug_assert!(matches!(
        kind,
        IndexedDbTaskKind::RequestSuccess | IndexedDbTaskKind::RequestError
    ));
    let owner = indexed_db_typed_execution_owner(scope, request)
        .expect("IDB request task wrapper should have typed owner state");
    let storage_scope = indexed_db_typed_storage_scope(scope, request);
    let id = register_indexed_db_task_with_owner(scope, task, kind, owner, storage_scope);
    let payload = IndexedDbRequestDispatchTaskPayload::new(scope, request);
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    table
        .borrow_mut()
        .request_dispatch_tasks
        .insert(id, payload);
}

pub(super) fn indexed_db_request_dispatch_task_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let id = indexed_db_typed_task_id(scope, task)?;
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    let table = table.borrow();
    let payload = table.request_dispatch_tasks.get(&id)?;
    let request = v8::Local::new(scope, &payload.request);
    v8::Local::<v8::Object>::try_from(request).ok()
}

pub(super) fn register_indexed_db_open_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
    database: v8::Local<'s, v8::Object>,
    transaction: v8::Local<'s, v8::Object>,
    old_version: u64,
    new_version: u64,
) {
    let owner = indexed_db_typed_execution_owner(scope, request)
        .expect("IDB open task should have typed owner state");
    let storage_scope = indexed_db_typed_storage_scope(scope, request);
    let id = register_indexed_db_task_with_owner(
        scope,
        task,
        IndexedDbTaskKind::Open,
        owner,
        storage_scope,
    );
    let payload = IndexedDbOpenTaskPayload::new(
        scope,
        request,
        database,
        transaction,
        old_version,
        new_version,
    );
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    table.borrow_mut().open_tasks.insert(id, payload);
}

type IndexedDbOpenTaskPayloadLocals<'s> = (
    v8::Local<'s, v8::Object>,
    v8::Local<'s, v8::Object>,
    v8::Local<'s, v8::Object>,
    u64,
    u64,
);

pub(super) fn indexed_db_open_task_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) -> Option<IndexedDbOpenTaskPayloadLocals<'s>> {
    let id = indexed_db_typed_task_id(scope, task)?;
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    let table = table.borrow();
    let payload = table.open_tasks.get(&id)?;
    let request = v8::Local::new(scope, &payload.request);
    let database = v8::Local::new(scope, &payload.database);
    let transaction = v8::Local::new(scope, &payload.transaction);
    Some((
        v8::Local::<v8::Object>::try_from(request).ok()?,
        v8::Local::<v8::Object>::try_from(database).ok()?,
        v8::Local::<v8::Object>::try_from(transaction).ok()?,
        payload.old_version,
        payload.new_version,
    ))
}

fn register_indexed_db_blocked_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
    kind: IndexedDbTaskKind,
    request: v8::Local<'s, v8::Object>,
    origin: &str,
    name: &str,
    version: Option<u64>,
    old_version: u64,
    new_version: Option<u64>,
) {
    debug_assert!(matches!(
        kind,
        IndexedDbTaskKind::OpenBlocked | IndexedDbTaskKind::DeleteBlocked
    ));
    let owner = indexed_db_typed_execution_owner(scope, request)
        .expect("IDB blocked task should have typed owner state");
    let storage_scope = indexed_db_typed_storage_scope(scope, request);
    let id = register_indexed_db_task_with_owner(scope, task, kind, owner, storage_scope);
    let payload = IndexedDbBlockedTaskPayload::new(
        scope,
        request,
        origin,
        name,
        version,
        old_version,
        new_version,
    );
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    table.borrow_mut().blocked_tasks.insert(id, payload);
}

pub(super) fn register_indexed_db_blocked_open_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
    origin: &str,
    name: &str,
    version: Option<u64>,
    old_version: u64,
    new_version: u64,
) {
    register_indexed_db_blocked_task(
        scope,
        task,
        IndexedDbTaskKind::OpenBlocked,
        request,
        origin,
        name,
        version,
        old_version,
        Some(new_version),
    );
}

pub(super) fn register_indexed_db_blocked_delete_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
    origin: &str,
    name: &str,
    old_version: u64,
) {
    register_indexed_db_blocked_task(
        scope,
        task,
        IndexedDbTaskKind::DeleteBlocked,
        request,
        origin,
        name,
        None,
        old_version,
        None,
    );
}

pub(super) fn indexed_db_blocked_task_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) -> Option<IndexedDbBlockedTaskPayloadLocals<'s>> {
    let id = indexed_db_typed_task_id(scope, task)?;
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    let table = table.borrow();
    let payload = table.blocked_tasks.get(&id)?;
    let request = v8::Local::new(scope, &payload.request);
    Some(IndexedDbBlockedTaskPayloadLocals {
        request: v8::Local::<v8::Object>::try_from(request).ok()?,
        origin: payload.origin.clone(),
        name: payload.name.clone(),
        version: payload.version,
        old_version: payload.old_version,
        new_version: payload.new_version,
    })
}

pub(super) fn register_indexed_db_transaction_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
    kind: IndexedDbTaskKind,
    transaction: v8::Local<'s, v8::Object>,
) {
    debug_assert!(matches!(
        kind,
        IndexedDbTaskKind::TransactionStart
            | IndexedDbTaskKind::TransactionCommit
            | IndexedDbTaskKind::TransactionAbort
    ));
    let owner = indexed_db_typed_execution_owner(scope, transaction)
        .expect("IDB transaction task should have typed owner state");
    let storage_scope = indexed_db_typed_storage_scope(scope, transaction);
    let id = register_indexed_db_task_with_owner(scope, task, kind, owner, storage_scope);
    let payload = IndexedDbTransactionTaskPayload::new(scope, transaction);
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    table.borrow_mut().transaction_tasks.insert(id, payload);
}

pub(super) fn indexed_db_transaction_task_transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let id = indexed_db_typed_task_id(scope, task)?;
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    let table = table.borrow();
    let payload = table.transaction_tasks.get(&id)?;
    let transaction = v8::Local::new(scope, &payload.transaction);
    v8::Local::<v8::Object>::try_from(transaction).ok()
}

pub(super) fn indexed_db_typed_task_storage_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) -> Option<IndexedDbStorageScope> {
    indexed_db_typed_task_state(scope, task).and_then(|state| state.storage_scope)
}

pub(super) fn unregister_indexed_db_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) {
    let Some(id) = indexed_db_typed_task_id(scope, task) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    let mut table = table.borrow_mut();
    table.tasks.remove(&id);
    table.databases_settle_tasks.remove(&id);
    table.request_dispatch_tasks.remove(&id);
    table.open_tasks.remove(&id);
    table.blocked_tasks.remove(&id);
    table.transaction_tasks.remove(&id);
}

pub(super) fn replace_indexed_db_database_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    metadata: impl IntoIterator<Item = IndexedDbObjectStoreMetadata>,
) -> Option<()> {
    let id = indexed_db_typed_state_id(scope, database)?;
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    let mut table = table.borrow_mut();
    let database = table.databases.get_mut(&id)?;
    database.metadata = metadata
        .into_iter()
        .map(|store| (store.info.name.clone(), store))
        .collect();
    Some(())
}

pub(super) fn indexed_db_database_store_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    store_name: &str,
) -> Option<IndexedDbObjectStoreMetadata> {
    let id = indexed_db_typed_state_id(scope, database)?;
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    table
        .borrow()
        .databases
        .get(&id)?
        .metadata
        .get(store_name)
        .cloned()
}

pub(super) fn set_indexed_db_database_store_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    metadata: IndexedDbObjectStoreMetadata,
) -> Option<()> {
    let id = indexed_db_typed_state_id(scope, database)?;
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    let mut table = table.borrow_mut();
    let database = table.databases.get_mut(&id)?;
    database
        .metadata
        .insert(metadata.info.name.clone(), metadata);
    Some(())
}

pub(super) fn remove_indexed_db_database_store_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    store_name: &str,
) -> Option<()> {
    let id = indexed_db_typed_state_id(scope, database)?;
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    let mut table = table.borrow_mut();
    let database = table.databases.get_mut(&id)?;
    database.metadata.remove(store_name);
    Some(())
}

pub(super) fn set_indexed_db_database_index_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    store_name: &str,
    info: IndexInfo,
) -> Option<()> {
    let id = indexed_db_typed_state_id(scope, database)?;
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    let mut table = table.borrow_mut();
    let database = table.databases.get_mut(&id)?;
    database.metadata.get_mut(store_name)?.set_index(info);
    Some(())
}

pub(super) fn remove_indexed_db_database_index_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    store_name: &str,
    index_name: &str,
) -> Option<()> {
    let id = indexed_db_typed_state_id(scope, database)?;
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    let mut table = table.borrow_mut();
    let database = table.databases.get_mut(&id)?;
    database
        .metadata
        .get_mut(store_name)?
        .remove_index(index_name);
    Some(())
}

pub(super) fn indexed_db_object_store_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
) -> Option<IndexedDbObjectStoreMetadata> {
    let id = indexed_db_typed_state_id(scope, store)?;
    let table = indexed_db_runtime_state_table_for_object(scope, store);
    table
        .borrow()
        .object_stores
        .get(&id)
        .map(|store| store.metadata.clone())
}

pub(super) fn indexed_db_object_store_transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let id = indexed_db_typed_state_id(scope, store)?;
    let table = indexed_db_runtime_state_table_for_object(scope, store);
    let table = table.borrow();
    let transaction = v8::Local::new(scope, &table.object_stores.get(&id)?.transaction);
    v8::Local::<v8::Object>::try_from(transaction).ok()
}

pub(super) fn indexed_db_object_store_database<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let id = indexed_db_typed_state_id(scope, store)?;
    let table = indexed_db_runtime_state_table_for_object(scope, store);
    let table = table.borrow();
    let database = v8::Local::new(scope, &table.object_stores.get(&id)?.database);
    v8::Local::<v8::Object>::try_from(database).ok()
}

pub(super) fn indexed_db_object_store_name(
    scope: &mut v8::PinScope<'_, '_>,
    store: v8::Local<'_, v8::Object>,
) -> Option<String> {
    let id = indexed_db_typed_state_id(scope, store)?;
    let table = indexed_db_runtime_state_table_for_object(scope, store);
    table
        .borrow()
        .object_stores
        .get(&id)
        .map(|store| store.name.clone())
}

pub(super) fn indexed_db_index_object_store<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let id = indexed_db_typed_state_id(scope, index)?;
    let table = indexed_db_runtime_state_table_for_object(scope, index);
    let table = table.borrow();
    let object_store = v8::Local::new(scope, &table.indexes.get(&id)?.object_store);
    v8::Local::<v8::Object>::try_from(object_store).ok()
}

pub(super) fn indexed_db_index_info(
    scope: &mut v8::PinScope<'_, '_>,
    index: v8::Local<'_, v8::Object>,
) -> Option<IndexInfo> {
    let id = indexed_db_typed_state_id(scope, index)?;
    let table = indexed_db_runtime_state_table_for_object(scope, index);
    table
        .borrow()
        .indexes
        .get(&id)
        .map(|index| index.info.clone())
}

pub(super) fn set_indexed_db_object_store_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
    metadata: IndexedDbObjectStoreMetadata,
) -> Option<()> {
    let id = indexed_db_typed_state_id(scope, store)?;
    let table = indexed_db_runtime_state_table_for_object(scope, store);
    let mut table = table.borrow_mut();
    let store = table.object_stores.get_mut(&id)?;
    store.name = metadata.info.name.clone();
    store.metadata = metadata;
    Some(())
}

pub(super) fn current_indexed_db_execution_owner(
    scope: &mut v8::PinScope<'_, '_>,
) -> IndexedDbExecutionOwner {
    let dispatch_scope = inferred_indexed_db_dispatch_scope(scope);
    if let Some(host_ptr) = crate::util::context_host_ptr_from_global_bridge(scope) {
        let host = unsafe { &*host_ptr };
        let execution_context = host
            .window_execution_context_identity_for_v8_context(scope, scope.get_current_context())
            .filter(|identity| identity.dispatch_scope() == dispatch_scope);
        if let Some(execution_context) = execution_context {
            return IndexedDbExecutionOwner::Window(execution_context);
        }
    }

    IndexedDbExecutionOwner::PendingWindow(dispatch_scope)
}

pub(super) fn indexed_db_execution_owner_for_object(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> IndexedDbExecutionOwner {
    if let Some(host_ptr) = crate::util::context_host_ptr_from_global_bridge(scope)
        && let Some(context) = object.get_creation_context(scope)
        && let Some(execution_context) =
            unsafe { &*host_ptr }.window_execution_context_identity_for_v8_context(scope, context)
    {
        return IndexedDbExecutionOwner::Window(execution_context);
    }
    current_indexed_db_execution_owner(scope)
}

fn inferred_indexed_db_dispatch_scope(
    scope: &mut v8::PinScope<'_, '_>,
) -> crate::native_bridge::OwnerDispatchScope {
    if let Some(popup_id) = crate::native_bridge::active_lightweight_popup_id(scope) {
        return crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id);
    }
    crate::context_bootstrap::current_child_browsing_context_handle_for_runtime_scope(scope)
        .map(crate::native_bridge::OwnerDispatchScope::Child)
        .unwrap_or(crate::native_bridge::OwnerDispatchScope::Top)
}

fn register_indexed_db_task_with_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
    kind: IndexedDbTaskKind,
    owner: IndexedDbExecutionOwner,
    storage_scope: Option<IndexedDbStorageScope>,
) -> IndexedDbTaskId {
    if let Some(storage_scope) = storage_scope.as_ref() {
        debug_assert!(storage_scope.has_explicit_partition_boundary());
    }
    let existing_id = indexed_db_typed_task_id(scope, task);
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    let id = table.borrow_mut().upsert_task(
        existing_id,
        IndexedDbTaskState::new(kind, owner, storage_scope),
    );
    set_indexed_db_typed_task_id(scope, task, id);
    id
}

fn indexed_db_typed_wrapper_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
) -> Option<IndexedDbWrapperState> {
    let id = indexed_db_typed_state_id(scope, wrapper)?;
    let table = indexed_db_runtime_state_table_for_object(scope, wrapper);
    table.borrow().wrapper(id).cloned()
}

fn indexed_db_typed_task_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) -> Option<IndexedDbTaskState> {
    let id = indexed_db_typed_task_id(scope, task)?;
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    table.borrow().task(id).cloned()
}

pub(super) fn indexed_db_typed_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let id = indexed_db_typed_state_id(scope, object)?;
    let table = indexed_db_runtime_state_table_for_object(scope, object);
    let table = table.borrow();
    if let Some(request) = table.requests.get(&id) {
        return indexed_db_typed_request_slot_value(scope, request, key);
    }
    if let Some(transaction) = table.transactions.get(&id) {
        return indexed_db_typed_transaction_slot_value(scope, transaction, key);
    }
    if let Some(database) = table.databases.get(&id) {
        return indexed_db_typed_database_slot_value(scope, database, key);
    }
    if let Some(cursor) = table.cursors.get(&id) {
        return indexed_db_typed_cursor_slot_value(scope, cursor, key);
    }
    if let Some(store) = table.object_stores.get(&id) {
        return indexed_db_typed_object_store_slot_value(scope, store, key);
    }
    if let Some(index) = table.indexes.get(&id) {
        return indexed_db_typed_index_slot_value(scope, index, key);
    }
    if let Some(key_range) = table.key_ranges.get(&id) {
        return indexed_db_typed_key_range_slot_value(scope, key_range, key);
    }
    None
}

pub(super) fn set_indexed_db_typed_slot_value(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &str,
    value: v8::Local<'_, v8::Value>,
) -> bool {
    let Some(id) = indexed_db_typed_state_id(scope, object) else {
        return false;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, object);
    let mut table = table.borrow_mut();
    if let Some(request) = table.requests.get_mut(&id) {
        return set_indexed_db_typed_request_slot_value(scope, request, key, value);
    }
    if let Some(transaction) = table.transactions.get_mut(&id) {
        return set_indexed_db_typed_transaction_slot_value(scope, transaction, key, value);
    }
    if let Some(database) = table.databases.get_mut(&id) {
        return set_indexed_db_typed_database_slot_value(scope, database, key, value);
    }
    if let Some(cursor) = table.cursors.get_mut(&id) {
        return set_indexed_db_typed_cursor_slot_value(scope, cursor, key, value);
    }
    if let Some(store) = table.object_stores.get_mut(&id) {
        return set_indexed_db_typed_object_store_slot_value(scope, store, key, value);
    }
    if let Some(index) = table.indexes.get_mut(&id) {
        return set_indexed_db_typed_index_slot_value(index, key, value);
    }
    if let Some(key_range) = table.key_ranges.get_mut(&id) {
        return set_indexed_db_typed_key_range_slot_value(key_range, key, value);
    }
    false
}

fn indexed_db_typed_request_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: &IndexedDbRequestLifecycleState,
    key: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    match key {
        INDEXED_DB_REQUEST_SOURCE_SLOT => Some(v8::Local::new(scope, &request.source)),
        INDEXED_DB_REQUEST_TRANSACTION_SLOT => Some(v8::Local::new(scope, &request.transaction)),
        INDEXED_DB_REQUEST_READY_STATE_SLOT => {
            v8_string(scope, &request.ready_state).map(Into::into)
        }
        INDEXED_DB_REQUEST_RESULT_SLOT => Some(v8::Local::new(scope, &request.result)),
        INDEXED_DB_REQUEST_ERROR_SLOT => Some(v8::Local::new(scope, &request.error)),
        INDEXED_DB_REQUEST_BLOCKED_DISPATCHED_SLOT => {
            Some(v8::Boolean::new(scope, request.blocked_dispatched).into())
        }
        INDEXED_DB_PENDING_RESULT_SLOT => request
            .pending_result
            .as_ref()
            .map(|value| v8::Local::new(scope, value)),
        INDEXED_DB_PENDING_ERROR_SLOT => request
            .pending_error
            .as_ref()
            .map(|value| v8::Local::new(scope, value)),
        INDEXED_DB_PENDING_CURSOR_SLOT => request
            .pending_cursor
            .as_ref()
            .map(|value| v8::Local::new(scope, value)),
        INDEXED_DB_PENDING_CURSOR_POSITION_SLOT => request
            .pending_cursor_position
            .map(|position| v8::Number::new(scope, position).into()),
        _ => None,
    }
}

fn set_indexed_db_typed_request_slot_value(
    scope: &mut v8::PinScope<'_, '_>,
    request: &mut IndexedDbRequestLifecycleState,
    key: &str,
    value: v8::Local<'_, v8::Value>,
) -> bool {
    match key {
        INDEXED_DB_REQUEST_SOURCE_SLOT => {
            request.source = v8::Global::new(scope, value);
            true
        }
        INDEXED_DB_REQUEST_TRANSACTION_SLOT => {
            request.transaction = v8::Global::new(scope, value);
            true
        }
        INDEXED_DB_REQUEST_READY_STATE_SLOT => {
            request.ready_state = value
                .to_string(scope)
                .map(|value| value.to_rust_string_lossy(scope))
                .unwrap_or_default();
            true
        }
        INDEXED_DB_REQUEST_RESULT_SLOT => {
            request.result = v8::Global::new(scope, value);
            true
        }
        INDEXED_DB_REQUEST_ERROR_SLOT => {
            request.error = v8::Global::new(scope, value);
            true
        }
        INDEXED_DB_REQUEST_BLOCKED_DISPATCHED_SLOT => {
            request.blocked_dispatched = value.boolean_value(scope);
            true
        }
        INDEXED_DB_PENDING_RESULT_SLOT => {
            request.pending_result = Some(v8::Global::new(scope, value));
            true
        }
        INDEXED_DB_PENDING_ERROR_SLOT => {
            request.pending_error = Some(v8::Global::new(scope, value));
            true
        }
        INDEXED_DB_PENDING_CURSOR_SLOT => {
            request.pending_cursor = Some(v8::Global::new(scope, value));
            true
        }
        INDEXED_DB_PENDING_CURSOR_POSITION_SLOT => {
            request.pending_cursor_position = value.number_value(scope);
            true
        }
        _ => false,
    }
}

fn indexed_db_typed_transaction_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: &IndexedDbTransactionLifecycleState,
    key: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    match key {
        INDEXED_DB_TRANSACTION_HANDLE_SLOT => transaction
            .handle
            .map(|handle| v8::Number::new(scope, handle.into_raw() as f64).into()),
        INDEXED_DB_TRANSACTION_ACTIVE_SLOT => {
            Some(v8::Boolean::new(scope, transaction.active).into())
        }
        INDEXED_DB_TRANSACTION_FINISHED_SLOT => {
            Some(v8::Boolean::new(scope, transaction.finished).into())
        }
        INDEXED_DB_TRANSACTION_ABORTED_SLOT => {
            Some(v8::Boolean::new(scope, transaction.aborted).into())
        }
        INDEXED_DB_TRANSACTION_STARTED_SLOT => {
            Some(v8::Boolean::new(scope, transaction.started).into())
        }
        INDEXED_DB_TRANSACTION_START_SCHEDULED_SLOT => {
            Some(v8::Boolean::new(scope, transaction.start_scheduled).into())
        }
        INDEXED_DB_TRANSACTION_ABORT_DISPATCHED_SLOT => {
            Some(v8::Boolean::new(scope, transaction.abort_dispatched).into())
        }
        INDEXED_DB_TRANSACTION_PENDING_SLOT => {
            Some(v8::Number::new(scope, transaction.pending as f64).into())
        }
        INDEXED_DB_TRANSACTION_COMMIT_SCHEDULED_SLOT => {
            Some(v8::Boolean::new(scope, transaction.commit_scheduled).into())
        }
        INDEXED_DB_TRANSACTION_DB_KEY_SLOT => transaction
            .db_key
            .as_ref()
            .and_then(|value| v8_string(scope, value))
            .map(Into::into),
        _ => None,
    }
}

fn set_indexed_db_typed_transaction_slot_value(
    scope: &mut v8::PinScope<'_, '_>,
    transaction: &mut IndexedDbTransactionLifecycleState,
    key: &str,
    value: v8::Local<'_, v8::Value>,
) -> bool {
    match key {
        INDEXED_DB_TRANSACTION_HANDLE_SLOT => {
            transaction.handle = value
                .number_value(scope)
                .map(|value| TransactionHandle::from_raw(value as u64));
            true
        }
        INDEXED_DB_TRANSACTION_ACTIVE_SLOT => {
            transaction.active = value.boolean_value(scope);
            true
        }
        INDEXED_DB_TRANSACTION_FINISHED_SLOT => {
            transaction.finished = value.boolean_value(scope);
            true
        }
        INDEXED_DB_TRANSACTION_ABORTED_SLOT => {
            transaction.aborted = value.boolean_value(scope);
            true
        }
        INDEXED_DB_TRANSACTION_STARTED_SLOT => {
            transaction.started = value.boolean_value(scope);
            true
        }
        INDEXED_DB_TRANSACTION_START_SCHEDULED_SLOT => {
            transaction.start_scheduled = value.boolean_value(scope);
            true
        }
        INDEXED_DB_TRANSACTION_ABORT_DISPATCHED_SLOT => {
            transaction.abort_dispatched = value.boolean_value(scope);
            true
        }
        INDEXED_DB_TRANSACTION_PENDING_SLOT => {
            transaction.pending = value.number_value(scope).unwrap_or(0.0).max(0.0).floor() as u32;
            true
        }
        INDEXED_DB_TRANSACTION_COMMIT_SCHEDULED_SLOT => {
            transaction.commit_scheduled = value.boolean_value(scope);
            true
        }
        INDEXED_DB_TRANSACTION_DB_KEY_SLOT => {
            transaction.db_key = if value.is_null_or_undefined() {
                None
            } else {
                value
                    .to_string(scope)
                    .map(|value| value.to_rust_string_lossy(scope))
            };
            true
        }
        _ => false,
    }
}

fn create_database_metadata_object_from_typed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    metadata: &BTreeMap<String, IndexedDbObjectStoreMetadata>,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = new_null_prototype_object(scope);
    for (store_name, store) in metadata {
        let descriptor = create_object_store_descriptor_from_typed(scope, store)?;
        set_indexed_db_internal_object_property(scope, object, store_name, descriptor.into());
    }
    Some(object)
}

fn create_object_store_descriptor_from_typed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    metadata: &IndexedDbObjectStoreMetadata,
) -> Option<v8::Local<'s, v8::Object>> {
    create_object_store_descriptor_object(scope, metadata.info(), &metadata.indexes_in_name_order())
}

fn indexed_db_typed_database_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: &IndexedDbDatabaseLifecycleState,
    key: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    match key {
        INDEXED_DB_DATABASE_HANDLE_SLOT => {
            Some(v8::Number::new(scope, database.handle.into_raw() as f64).into())
        }
        INDEXED_DB_DATABASE_KEY_SLOT => v8_string(scope, &database.database_key).map(Into::into),
        INDEXED_DB_DATABASE_STORAGE_KEY_SLOT => {
            v8_string(scope, database.storage_scope.storage_key()).map(Into::into)
        }
        INDEXED_DB_DATABASE_CLOSED_SLOT => Some(v8::Boolean::new(scope, database.closed).into()),
        INDEXED_DB_DATABASE_METADATA_SLOT => {
            create_database_metadata_object_from_typed(scope, &database.metadata).map(Into::into)
        }
        INDEXED_DB_DATABASE_UPGRADE_TRANSACTION_SLOT => database
            .upgrade_transaction
            .as_ref()
            .map(|value| v8::Local::new(scope, value)),
        _ => None,
    }
}

fn set_indexed_db_typed_database_slot_value(
    scope: &mut v8::PinScope<'_, '_>,
    database: &mut IndexedDbDatabaseLifecycleState,
    key: &str,
    value: v8::Local<'_, v8::Value>,
) -> bool {
    match key {
        INDEXED_DB_DATABASE_HANDLE_SLOT => {
            database.handle =
                DatabaseHandle::from_raw(value.number_value(scope).unwrap_or(0.0) as u64);
            true
        }
        INDEXED_DB_DATABASE_KEY_SLOT => {
            database.database_key = value
                .to_string(scope)
                .map(|value| value.to_rust_string_lossy(scope))
                .unwrap_or_default();
            true
        }
        INDEXED_DB_DATABASE_STORAGE_KEY_SLOT => {
            database.storage_scope.storage_key = value
                .to_string(scope)
                .map(|value| value.to_rust_string_lossy(scope))
                .unwrap_or_default();
            true
        }
        INDEXED_DB_DATABASE_CLOSED_SLOT => {
            database.closed = value.boolean_value(scope);
            true
        }
        // Metadata is typed-state authoritative. Consume migrated slot writes here so they do not
        // fall through into V8 private storage and become a second metadata source.
        INDEXED_DB_DATABASE_METADATA_SLOT => true,
        INDEXED_DB_DATABASE_UPGRADE_TRANSACTION_SLOT => {
            database.upgrade_transaction = if value.is_null_or_undefined() {
                None
            } else {
                Some(v8::Global::new(scope, value))
            };
            true
        }
        _ => false,
    }
}

fn indexed_db_typed_cursor_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: &IndexedDbCursorLifecycleState,
    key: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    match key {
        INDEXED_DB_CURSOR_REQUEST_SLOT => Some(v8::Local::new(scope, &cursor.request)),
        INDEXED_DB_CURSOR_ENTRIES_SLOT => Some(v8::Local::new(scope, &cursor.entries)),
        INDEXED_DB_CURSOR_KEY_ONLY_SLOT => Some(v8::Boolean::new(scope, cursor.key_only).into()),
        INDEXED_DB_CURSOR_POSITION_SLOT => Some(v8::Number::new(scope, cursor.position).into()),
        _ => None,
    }
}

fn set_indexed_db_typed_cursor_slot_value(
    scope: &mut v8::PinScope<'_, '_>,
    cursor: &mut IndexedDbCursorLifecycleState,
    key: &str,
    value: v8::Local<'_, v8::Value>,
) -> bool {
    match key {
        INDEXED_DB_CURSOR_REQUEST_SLOT => {
            cursor.request = v8::Global::new(scope, value);
            true
        }
        INDEXED_DB_CURSOR_ENTRIES_SLOT => {
            cursor.entries = v8::Global::new(scope, value);
            true
        }
        INDEXED_DB_CURSOR_KEY_ONLY_SLOT => {
            cursor.key_only = value.boolean_value(scope);
            true
        }
        INDEXED_DB_CURSOR_POSITION_SLOT => {
            cursor.position = value.number_value(scope).unwrap_or(-1.0);
            true
        }
        _ => false,
    }
}

fn indexed_db_typed_object_store_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: &IndexedDbObjectStoreLifecycleState,
    key: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    match key {
        INDEXED_DB_OBJECT_STORE_NAME_SLOT => v8_string(scope, &store.name).map(Into::into),
        INDEXED_DB_OBJECT_STORE_METADATA_SLOT => {
            create_object_store_descriptor_from_typed(scope, &store.metadata).map(Into::into)
        }
        _ => None,
    }
}

fn set_indexed_db_typed_object_store_slot_value(
    scope: &mut v8::PinScope<'_, '_>,
    store: &mut IndexedDbObjectStoreLifecycleState,
    key: &str,
    value: v8::Local<'_, v8::Value>,
) -> bool {
    match key {
        INDEXED_DB_OBJECT_STORE_NAME_SLOT => {
            store.name = value
                .to_string(scope)
                .map(|value| value.to_rust_string_lossy(scope))
                .unwrap_or_default();
            true
        }
        // Metadata is typed-state authoritative. Consume migrated slot writes here so they do not
        // fall through into V8 private storage and become a second metadata source.
        INDEXED_DB_OBJECT_STORE_METADATA_SLOT => true,
        _ => false,
    }
}

fn indexed_db_typed_index_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: &IndexedDbIndexLifecycleState,
    key: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    match key {
        INDEXED_DB_INDEX_MARKER_SLOT => Some(v8::Boolean::new(scope, index.marker).into()),
        _ => None,
    }
}

fn set_indexed_db_typed_index_slot_value(
    index: &mut IndexedDbIndexLifecycleState,
    key: &str,
    value: v8::Local<'_, v8::Value>,
) -> bool {
    match key {
        INDEXED_DB_INDEX_MARKER_SLOT => {
            index.marker = value.is_true();
            true
        }
        _ => false,
    }
}

fn indexed_db_typed_key_range_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key_range: &IndexedDbKeyRangeLifecycleState,
    key: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    match key {
        INDEXED_DB_KEY_RANGE_MARKER_SLOT => Some(v8::Boolean::new(scope, key_range.marker).into()),
        _ => None,
    }
}

fn set_indexed_db_typed_key_range_slot_value(
    key_range: &mut IndexedDbKeyRangeLifecycleState,
    key: &str,
    value: v8::Local<'_, v8::Value>,
) -> bool {
    match key {
        INDEXED_DB_KEY_RANGE_MARKER_SLOT => {
            key_range.marker = value.is_true();
            true
        }
        _ => false,
    }
}

fn indexed_db_typed_state_id(
    scope: &mut v8::PinScope<'_, '_>,
    wrapper: v8::Local<'_, v8::Object>,
) -> Option<IndexedDbObjectId> {
    indexed_db_private_value(scope, wrapper, INDEXED_DB_TYPED_STATE_ID_SLOT)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .and_then(|value| {
            let (raw, lossless) = value.u64_value();
            (lossless && raw != 0).then_some(IndexedDbObjectId(raw))
        })
}

pub(super) fn indexed_db_typed_task_id(
    scope: &mut v8::PinScope<'_, '_>,
    task: v8::Local<'_, v8::Object>,
) -> Option<IndexedDbTaskId> {
    indexed_db_private_value(scope, task, INDEXED_DB_TYPED_TASK_ID_SLOT)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .and_then(|value| {
            let (raw, lossless) = value.u64_value();
            (lossless && raw != 0).then_some(IndexedDbTaskId(raw))
        })
}

fn set_indexed_db_typed_state_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
    id: IndexedDbObjectId,
) {
    let id = v8::BigInt::new_from_u64(scope, id.0);
    set_indexed_db_slot_value(scope, wrapper, INDEXED_DB_TYPED_STATE_ID_SLOT, id.into());
}

fn set_indexed_db_typed_task_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
    id: IndexedDbTaskId,
) {
    let id = v8::BigInt::new_from_u64(scope, id.0);
    set_indexed_db_slot_value(scope, task, INDEXED_DB_TYPED_TASK_ID_SLOT, id.into());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_state_table_updates_existing_wrapper_record() {
        let mut table = IndexedDbRuntimeStateTable::default();
        let initial = IndexedDbWrapperState::new(
            IndexedDbWrapperKind::Factory,
            IndexedDbExecutionOwner::without_execution_context(
                crate::native_bridge::OwnerDispatchScope::Top,
            ),
            None,
        );
        let id = table.upsert_wrapper(None, initial);

        let scope = IndexedDbStorageScope::new(
            "storage-key:v1;origin=https://example.test",
            "browser-context:test",
            "profile-partition:test",
        );
        let updated = IndexedDbWrapperState::new(
            IndexedDbWrapperKind::Database,
            IndexedDbExecutionOwner::without_execution_context(
                crate::native_bridge::OwnerDispatchScope::Top,
            ),
            Some(scope.clone()),
        );
        let updated_id = table.upsert_wrapper(Some(id), updated);

        assert_eq!(updated_id, id);
        assert_eq!(
            table
                .wrapper(id)
                .and_then(|state| state.storage_scope.as_ref())
                .map(IndexedDbStorageScope::storage_key),
            Some(scope.storage_key())
        );
        assert_eq!(
            table
                .wrapper(id)
                .and_then(|state| state.storage_scope.as_ref())
                .map(IndexedDbStorageScope::browser_context_id),
            Some("browser-context:test")
        );
        assert_eq!(
            table
                .wrapper(id)
                .and_then(|state| state.storage_scope.as_ref())
                .map(IndexedDbStorageScope::profile_partition_id),
            Some("profile-partition:test")
        );
    }

    #[test]
    fn runtime_state_table_stores_task_owner_and_storage_scope() {
        let mut table = IndexedDbRuntimeStateTable::default();
        let storage_scope = IndexedDbStorageScope::new(
            "storage-key:v1;origin=https://task.example",
            "browser-context:task",
            "profile-partition:task",
        );
        let id = table.upsert_task(
            None,
            IndexedDbTaskState::new(
                IndexedDbTaskKind::OpenBlocked,
                IndexedDbExecutionOwner::without_execution_context(
                    crate::native_bridge::OwnerDispatchScope::Top,
                ),
                Some(storage_scope.clone()),
            ),
        );

        assert_eq!(
            table.task(id).map(IndexedDbTaskState::owner_for_dispatch),
            Some(crate::native_bridge::OwnerDispatchScope::Top)
        );
        assert_eq!(
            table
                .task(id)
                .and_then(|state| state.storage_scope.as_ref())
                .map(IndexedDbStorageScope::storage_key),
            Some(storage_scope.storage_key())
        );
        assert_eq!(
            table
                .task(id)
                .and_then(|state| state.storage_scope.as_ref())
                .map(IndexedDbStorageScope::browser_context_id),
            Some("browser-context:task")
        );
        assert_eq!(
            table
                .task(id)
                .and_then(|state| state.storage_scope.as_ref())
                .map(IndexedDbStorageScope::profile_partition_id),
            Some("profile-partition:task")
        );
    }

    #[test]
    fn object_store_metadata_tracks_index_lifecycle() {
        let mut metadata = IndexedDbObjectStoreMetadata::new(
            ObjectStoreInfo {
                name: "posts".to_owned(),
                key_path: None,
                auto_increment: false,
                index_names: Vec::new(),
            },
            [],
        );
        let index = IndexInfo {
            name: "by-tag".to_owned(),
            key_path: KeyPath::String("tag".to_owned()),
            unique: true,
            multi_entry: false,
        };

        metadata.set_index(index.clone());

        assert_eq!(metadata.info().index_names, ["by-tag"]);
        assert_eq!(metadata.index("by-tag"), Some(&index));
        assert_eq!(
            metadata.indexes_in_name_order(),
            std::slice::from_ref(&index)
        );

        metadata.remove_index("by-tag");

        assert!(metadata.info().index_names.is_empty());
        assert!(metadata.index("by-tag").is_none());
        assert!(metadata.indexes_in_name_order().is_empty());
    }
}
