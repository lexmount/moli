use super::*;
use crate::util::RealmValueHandle;
use moli_storage_service::StorageBucketIdentity;
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

mod mirror;
pub(super) use mirror::IndexedDbObjectStoreMetadata;
#[cfg(test)]
use mirror::IndexedDbObjectStoreMetadataData;
use mirror::{StoreMap, edit_shared, missing_mirror};
mod collection;
mod cursor;
use collection::*;
pub(super) use collection::{INDEX_OBJECT_STORE, OBJECT_STORE_DATABASE, OBJECT_STORE_TRANSACTION};
pub(in crate::context_bootstrap::indexed_db) use cursor::*;
mod upgrade;
pub(super) use upgrade::*;
pub(super) mod factory;
pub(super) mod sql;

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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IndexedDbTaskKind {
    SqlCompletion,
    FactoryRequest,
    RequestSuccess,
    RequestError,
    Open,
    OpenSuccess,
    VersionChange,
    DatabasesSettle,
    TransactionStart,
    TransactionCommit,
    TransactionAbort,
    TransactionOperationError,
    TransactionCancel,
    DatabaseForcedClose,
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
            | IndexedDbWrapperKind::Index => self.owner.dispatch_scope(),
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
            IndexedDbTaskKind::SqlCompletion
            | IndexedDbTaskKind::FactoryRequest
            | IndexedDbTaskKind::RequestSuccess
            | IndexedDbTaskKind::RequestError
            | IndexedDbTaskKind::Open
            | IndexedDbTaskKind::OpenSuccess
            | IndexedDbTaskKind::VersionChange
            | IndexedDbTaskKind::DatabasesSettle
            | IndexedDbTaskKind::TransactionStart
            | IndexedDbTaskKind::TransactionCommit
            | IndexedDbTaskKind::TransactionAbort
            | IndexedDbTaskKind::TransactionOperationError
            | IndexedDbTaskKind::TransactionCancel
            | IndexedDbTaskKind::DatabaseForcedClose => self.owner.dispatch_scope(),
        }
    }
}

struct IndexedDbDatabasesSettleTaskPayload {
    resolver: RealmValueHandle,
    value: RealmValueHandle,
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
            resolver: RealmValueHandle::new(scope, resolver),
            value: RealmValueHandle::new(scope, value),
            reject,
        }
    }
}

struct IndexedDbRequestDispatchTaskPayload {
    request: RealmValueHandle,
}

impl IndexedDbRequestDispatchTaskPayload {
    fn new(scope: &mut v8::PinScope<'_, '_>, request: v8::Local<'_, v8::Object>) -> Self {
        let request: v8::Local<'_, v8::Value> = request.into();
        Self {
            request: RealmValueHandle::new(scope, request),
        }
    }
}

struct IndexedDbOpenTaskPayload {
    request: RealmValueHandle,
    database: RealmValueHandle,
    transaction: RealmValueHandle,
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
            request: RealmValueHandle::new(scope, request),
            database: RealmValueHandle::new(scope, database),
            transaction: RealmValueHandle::new(scope, transaction),
            old_version,
            new_version,
        }
    }
}

struct IndexedDbVersionChangeTaskPayload {
    completion: Option<moli_indexeddb::sqlite::service::VersionChangeAcknowledgement>,
    database: v8::Global<v8::Object>,
    old_version: u64,
    new_version: Option<u64>,
}

struct IndexedDbObjectTaskPayload {
    target: RealmValueHandle,
    failure: Option<(IndexedDbObjectId, IndexedDbError)>,
}

impl IndexedDbObjectTaskPayload {
    fn new(scope: &mut v8::PinScope<'_, '_>, target: v8::Local<'_, v8::Object>) -> Self {
        Self {
            target: RealmValueHandle::new(scope, target.into()),
            failure: None,
        }
    }
}

struct IndexedDbRequestLifecycleState {
    delete_old_version: Option<u64>,
    ready_state: String,
    blocked_dispatched: bool,
    pending_cursor_position: Option<f64>,
    error_pending_dispatch: bool,
    pending_cursor_page: Option<Rc<cursor::SqlCursorPage>>,
    connection_request: Option<moli_indexeddb::ConnectionRequestLease>,
}

struct IndexedDbTransactionLifecycleState {
    mode: TransactionMode,
    database: Option<RealmValueHandle>,
    upgrade_open_request: Option<RealmValueHandle>,
    handle: Option<TransactionHandle>,
    sql_client: Option<moli_indexeddb::sqlite::service::DatabaseClient>,
    sql_rollback: Option<sql::SqlTransactionRollback>,
    start_request: Option<moli_indexeddb::TransactionRequestLease>,
    active: bool,
    committing: bool,
    finished: bool,
    aborted: bool,
    started: bool,
    start_scheduled: bool,
    cancellation_scheduled: bool,
    abort_dispatched: bool,
    pending: u32,
    pending_errors: u32,
    commit_scheduled: bool,
    deactivation_scheduled: bool,
    operation_in_flight: bool,
    operations: std::collections::VecDeque<IndexedDbPendingTransactionOperation>,
}

impl IndexedDbTransactionLifecycleState {
    fn new(
        database: RealmValueHandle,
        handle: Option<TransactionHandle>,
        mode: TransactionMode,
        started: bool,
    ) -> Self {
        Self {
            mode,
            database: Some(database),
            upgrade_open_request: None,
            handle,
            sql_client: None,
            sql_rollback: None,
            start_request: None,
            active: true,
            committing: false,
            finished: false,
            aborted: false,
            started,
            start_scheduled: false,
            cancellation_scheduled: false,
            abort_dispatched: false,
            pending: 0,
            pending_errors: 0,
            commit_scheduled: false,
            deactivation_scheduled: false,
            operation_in_flight: false,
            operations: Default::default(),
        }
    }
}

struct IndexedDbDatabaseBackend {
    client: moli_indexeddb::sqlite::service::DatabaseClient,
    connection: Option<moli_indexeddb::sqlite::service::DatabaseConnection>,
    forced_close: Option<moli_indexeddb::DatabaseCloseSignal>,
}

impl IndexedDbDatabaseBackend {
    fn is_forced_closed(&self) -> bool {
        self.client.is_closed()
            || self
                .forced_close
                .as_ref()
                .is_some_and(moli_indexeddb::DatabaseCloseSignal::is_forced_closed)
    }
}

struct IndexedDbDatabaseLifecycleState {
    wrapper: v8::Weak<v8::Object>,
    backend: IndexedDbDatabaseBackend,
    forced_close_scheduled: bool,
    handle: DatabaseHandle,
    storage_scope: IndexedDbStorageScope,
    closed: bool,
    mirror: std::rc::Weak<RefCell<IndexedDbDatabaseMirror>>,
    upgrade_transaction: Option<RealmValueHandle>,
    handle_tables: Vec<std::rc::Weak<RefCell<IndexedDbRuntimeStateTable>>>,
}

#[derive(Default)]
struct IndexedDbDatabaseMirror {
    metadata: Option<Rc<StoreMap>>,
    sql_schema: Option<std::sync::Weak<moli_indexeddb::sqlite::StoreDefinitions>>,
    upgrade_metadata: Option<IndexedDbUpgradeMetadata>,
}

impl IndexedDbDatabaseLifecycleState {
    fn new(
        scope: &mut v8::PinScope<'_, '_>,
        wrapper: v8::Local<'_, v8::Object>,
        backend: IndexedDbDatabaseBackend,
        handle: DatabaseHandle,
        storage_scope: IndexedDbStorageScope,
    ) -> Self {
        Self {
            wrapper: v8::Weak::new(scope, wrapper),
            backend,
            forced_close_scheduled: false,
            handle,
            storage_scope,
            closed: false,
            mirror: std::rc::Weak::new(),
            upgrade_transaction: None,
            handle_tables: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub(super) struct IndexedDbCursorLifecycleState {
    pub(super) entries: Rc<[CursorSnapshotEntry]>,
    pub(super) position: Option<usize>,
    pub(super) got_value: bool,
    pub(super) direction: CursorDirection,
    pub(super) key_only: bool,
    pub(super) sql: Option<Rc<cursor::SqlCursorPage>>,
}

struct IndexedDbObjectStoreLifecycleState {
    transaction: v8::Weak<v8::Object>,
    database: v8::Weak<v8::Object>,
    name_override: Option<moli_indexeddb::IndexedDbName>,
    metadata: IndexedDbObjectStoreMetadata,
    deleted: bool,
}

impl IndexedDbObjectStoreLifecycleState {
    fn name(&self) -> &moli_indexeddb::IndexedDbName {
        self.name_override
            .as_ref()
            .unwrap_or(&self.metadata.info.name)
    }

    fn new(
        scope: &mut v8::PinScope<'_, '_>,
        wrapper: v8::Local<'_, v8::Object>,
        transaction: v8::Local<'_, v8::Object>,
        database: v8::Local<'_, v8::Object>,
        metadata: IndexedDbObjectStoreMetadata,
    ) -> Self {
        set_private_value(scope, wrapper, OBJECT_STORE_TRANSACTION, transaction.into());
        set_private_value(scope, wrapper, OBJECT_STORE_DATABASE, database.into());
        Self {
            transaction: v8::Weak::new(scope, transaction),
            database: v8::Weak::new(scope, database),
            name_override: None,
            metadata,
            deleted: false,
        }
    }
}

struct IndexedDbIndexLifecycleState {
    object_store: v8::Weak<v8::Object>,
    info: Rc<IndexInfo>,
    marker: bool,
    deleted: bool,
    created_in_upgrade: bool,
}

impl IndexedDbIndexLifecycleState {
    fn new(
        scope: &mut v8::PinScope<'_, '_>,
        index: v8::Local<'_, v8::Object>,
        object_store: v8::Local<'_, v8::Object>,
        info: Rc<IndexInfo>,
        created_in_upgrade: bool,
    ) -> Self {
        set_private_value(scope, index, INDEX_OBJECT_STORE, object_store.into());
        Self {
            object_store: v8::Weak::new(scope, object_store),
            info,
            marker: true,
            deleted: false,
            created_in_upgrade,
        }
    }
}

#[derive(Default)]
pub(super) struct IndexedDbRuntimeStateTable {
    next_id: u64,
    wrappers: BTreeMap<IndexedDbObjectId, IndexedDbWrapperState>,
    tasks: BTreeMap<IndexedDbTaskId, IndexedDbTaskState>,
    sql_tasks: BTreeMap<IndexedDbTaskId, sql::SqlTaskPayload>,
    factory_requests: BTreeMap<IndexedDbTaskId, Rc<RefCell<factory::FactoryRequest>>>,
    databases_settle_tasks: BTreeMap<IndexedDbTaskId, IndexedDbDatabasesSettleTaskPayload>,
    request_dispatch_tasks: BTreeMap<IndexedDbTaskId, IndexedDbRequestDispatchTaskPayload>,
    open_tasks: BTreeMap<IndexedDbTaskId, IndexedDbOpenTaskPayload>,
    version_change_tasks: BTreeMap<IndexedDbTaskId, IndexedDbVersionChangeTaskPayload>,
    object_tasks: BTreeMap<IndexedDbTaskId, IndexedDbObjectTaskPayload>,
    requests: BTreeMap<IndexedDbObjectId, IndexedDbRequestLifecycleState>,
    transactions: BTreeMap<IndexedDbObjectId, IndexedDbTransactionLifecycleState>,
    databases: BTreeMap<IndexedDbObjectId, IndexedDbDatabaseLifecycleState>,
    // Cursor payloads belong to their V8 wrappers, not to the realm registry.
    cursors: BTreeMap<IndexedDbObjectId, std::rc::Weak<RefCell<IndexedDbCursorLifecycleState>>>,
    collectible_wrappers: BTreeMap<IndexedDbObjectId, v8::Weak<v8::Object>>,
    collected_wrappers: Rc<RefCell<Vec<IndexedDbObjectId>>>,
    object_stores:
        BTreeMap<IndexedDbObjectId, std::rc::Weak<RefCell<IndexedDbObjectStoreLifecycleState>>>,
    indexes: BTreeMap<IndexedDbObjectId, std::rc::Weak<RefCell<IndexedDbIndexLifecycleState>>>,
}

impl IndexedDbRuntimeStateTable {
    fn upsert_wrapper(
        &mut self,
        id: Option<IndexedDbObjectId>,
        state: IndexedDbWrapperState,
    ) -> IndexedDbObjectId {
        let id = id.unwrap_or_else(|| {
            self.next_id = self
                .next_id
                .checked_add(1)
                .expect("IndexedDB runtime object id space exhausted");
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
            self.next_id = self
                .next_id
                .checked_add(1)
                .expect("IndexedDB runtime task id space exhausted");
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

#[cfg(test)]
pub(crate) fn indexed_db_has_in_flight_sql_tasks_for_test(
    scope: &mut v8::PinScope<'_, '_>,
) -> bool {
    scope
        .get_current_context()
        .get_slot::<RefCell<IndexedDbRuntimeStateTable>>()
        .is_some_and(|table| {
            let table = table.borrow();
            !table.sql_tasks.is_empty()
                || table.databases.values().any(|database| {
                    let Some(connection) = &database.backend.connection else {
                        return false;
                    };
                    connection.is_closed()
                        && !connection.is_close_complete()
                        && !connection.client().is_closed()
                        && table.factory_requests.values().any(|factory| {
                            factory
                                .borrow()
                                .waits_for_close_for_test(connection.client())
                        })
                })
                || table.factory_requests.iter().any(|(id, request)| {
                    factory::factory_task_is_ready(&table, *id)
                        || request.borrow().has_native_close_receipt_for_test()
                })
        })
}

#[cfg(test)]
pub(crate) fn indexed_db_has_queued_task_kind_for_test(
    scope: &mut v8::PinScope<'_, '_>,
    kind: IndexedDbTaskKind,
) -> bool {
    scope
        .get_current_context()
        .get_slot::<RefCell<IndexedDbRuntimeStateTable>>()
        .is_some_and(|table| table.borrow().tasks.values().any(|task| task.kind == kind))
}

fn ensure_indexed_db_runtime_state_table_for_context(
    context: v8::Local<'_, v8::Context>,
) -> Rc<RefCell<IndexedDbRuntimeStateTable>> {
    if let Some(table) = context.get_slot::<RefCell<IndexedDbRuntimeStateTable>>() {
        // Nested state lookups may already hold a table borrow. Deferring this
        // small metadata sweep is safe; finalizers release large payloads directly.
        if let Ok(mut state) = table.try_borrow_mut() {
            state.sweep_collected_wrappers();
        }
        return table;
    }
    let table = Rc::new(RefCell::new(IndexedDbRuntimeStateTable::default()));
    let _ = context.set_slot(table.clone());
    table
}

pub(crate) fn retain_indexed_db_state_in_retired_realm(scope: &mut v8::PinScope<'_, '_>) {
    let context = scope.get_current_context();
    retire_indexed_db_context(context);
    let Some(table) = context.get_slot::<RefCell<IndexedDbRuntimeStateTable>>() else {
        return;
    };
    {
        let mut state = table.borrow_mut();
        state.tasks.clear();
        state.sql_tasks.clear();
        state.databases_settle_tasks.clear();
        state.request_dispatch_tasks.clear();
        state.open_tasks.clear();
        state.object_tasks.clear();
        for transaction in state.transactions.values_mut() {
            transaction.active = false;
            for value in [
                &mut transaction.database,
                &mut transaction.upgrade_open_request,
            ]
            .into_iter()
            .flatten()
            {
                value.retain_in_realm(scope);
            }
            transaction.operations.clear();
            transaction.operation_in_flight = false;
            transaction.sql_rollback = None;
        }
        for database in state.databases.values_mut() {
            database.closed = true;
            if let Some(value) = &mut database.upgrade_transaction {
                value.retain_in_realm(scope);
            }
        }
    }
    crate::util::retain_context_v8_handle_state_for_safe_release(context, table);
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
    // The private edges are traced from this request, including when a result
    // or source points back to it. No native Global roots that cycle.
    for (slot, value) in [
        (INDEXED_DB_REQUEST_SOURCE_SLOT, source),
        (INDEXED_DB_REQUEST_TRANSACTION_SLOT, transaction),
        (INDEXED_DB_REQUEST_RESULT_SLOT, v8::undefined(scope).into()),
        (INDEXED_DB_REQUEST_ERROR_SLOT, v8::null(scope).into()),
    ] {
        set_private_value(scope, request, slot, value);
    }
    let table = indexed_db_runtime_state_table_for_object(scope, request);
    table.borrow_mut().requests.insert(
        id,
        IndexedDbRequestLifecycleState {
            delete_old_version: None,
            ready_state: "pending".to_owned(),
            blocked_dispatched,
            pending_cursor_position: None,
            error_pending_dispatch: false,
            pending_cursor_page: None,
            connection_request: None,
        },
    );
    track_collectible_wrapper(scope, request, id, &table, ());
}

pub(super) fn release_indexed_db_request_dispatch_refs<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) {
    let Some(id) = indexed_db_typed_state_id(scope, request) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, request);
    if let Some(state) = table.borrow_mut().requests.get_mut(&id) {
        state.pending_cursor_position = None;
        state.pending_cursor_page = None;
    }
    clear_pending_request_values(scope, request);
}

#[cfg(test)]
pub(crate) fn indexed_db_request_state_entries_for_test(
    scope: &mut v8::PinScope<'_, '_>,
    source: v8::Local<'_, v8::Object>,
) -> usize {
    let table = indexed_db_runtime_state_table_for_object(scope, source);
    let table = table.borrow();
    table.requests.len()
        + table
            .wrappers
            .values()
            .filter(|state| matches!(state.kind, IndexedDbWrapperKind::Request))
            .count()
}

pub(super) fn register_indexed_db_transaction_lifecycle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    database: v8::Local<'s, v8::Object>,
    handle: Option<TransactionHandle>,
    mode: TransactionMode,
    started: bool,
) {
    let Some(id) = indexed_db_typed_state_id(scope, transaction) else {
        return;
    };
    let state = IndexedDbTransactionLifecycleState::new(
        RealmValueHandle::new(scope, database.into()),
        handle,
        mode,
        started,
    );
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    table.borrow_mut().transactions.insert(id, state);
    track_indexed_db_handle_table(scope, database, &table);
}

pub(super) fn indexed_db_transaction_mode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) -> Option<TransactionMode> {
    let id = indexed_db_typed_state_id(scope, transaction)?;
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    table.borrow().transactions.get(&id).map(|state| state.mode)
}

pub(super) fn indexed_db_transaction_is_committing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(id) = indexed_db_typed_state_id(scope, transaction) else {
        return false;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    table
        .borrow()
        .transactions
        .get(&id)
        .is_some_and(|state| state.committing)
}

pub(super) fn request_indexed_db_transaction_commit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) -> Result<(), IndexedDbError> {
    let id = indexed_db_typed_state_id(scope, transaction)
        .ok_or_else(|| IndexedDbError::InvalidState("Transaction was released".into()))?;
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    let mut table = table.borrow_mut();
    let state = table
        .transactions
        .get_mut(&id)
        .ok_or_else(|| IndexedDbError::InvalidState("Transaction was released".into()))?;
    if !state.active || state.finished || state.aborted || state.committing {
        return Err(IndexedDbError::InvalidState(
            "The transaction is not active".into(),
        ));
    }
    state.committing = true;
    state.active = false;
    Ok(())
}

/// Admission, native operations, and request events are separate boundaries.
/// Explicit commit drains accepted operations without waiting for success
/// events; automatic commit also waits for event dispatch and microtasks.
pub(super) fn indexed_db_transaction_can_commit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(id) = indexed_db_typed_state_id(scope, transaction) else {
        return false;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    table.borrow().transactions.get(&id).is_some_and(|state| {
        state.started
            && !state.finished
            && !state.aborted
            && !state.operation_in_flight
            && state.operations.is_empty()
            && (state.committing || (!state.active && state.pending == 0))
    })
}

pub(super) fn mark_indexed_db_transaction_committing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) {
    let Some(id) = indexed_db_typed_state_id(scope, transaction) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    if let Some(state) = table.borrow_mut().transactions.get_mut(&id) {
        state.committing = true;
        state.active = false;
    }
}

pub(super) fn indexed_db_transaction_has_pending_errors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(id) = indexed_db_typed_state_id(scope, transaction) else {
        return false;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    table
        .borrow()
        .transactions
        .get(&id)
        .is_some_and(|state| state.pending_errors != 0)
}

pub(super) fn set_indexed_db_request_error_pending<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    pending: bool,
) {
    let Some(transaction) = indexed_db_request_transaction_object(scope, request) else {
        return;
    };
    let Some(transaction_id) = indexed_db_typed_state_id(scope, transaction) else {
        return;
    };
    let Some(request_id) = indexed_db_typed_state_id(scope, request) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, request);
    let mut table = table.borrow_mut();
    let Some(request) = table.requests.get_mut(&request_id) else {
        return;
    };
    if request.error_pending_dispatch == pending {
        return;
    }
    request.error_pending_dispatch = pending;
    if let Some(transaction) = table.transactions.get_mut(&transaction_id) {
        if pending {
            transaction.pending_errors += 1;
        } else {
            transaction.pending_errors = transaction.pending_errors.saturating_sub(1);
        }
    }
}

pub(super) fn mark_indexed_db_transaction_started<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    handle: TransactionHandle,
) {
    let Some(id) = indexed_db_typed_state_id(scope, transaction) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    if let Some(state) = table.borrow_mut().transactions.get_mut(&id) {
        state.handle = Some(handle);
        state.started = true;
        state.start_scheduled = false;
    }
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
        if state.pending == 0 {
            state.committing = true;
            state.started
        } else {
            false
        }
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
    let database = transaction
        .database
        .as_ref()
        .map(|value| value.to_local(scope));
    transaction.database = None;
    transaction.upgrade_open_request = None;
    transaction.operations.clear();
    transaction.operation_in_flight = false;
    transaction.sql_rollback = None;
    drop(table);
    if let Some(database) = database.and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        crate::context_bootstrap::indexed_db::finish_indexed_db_database_close(scope, database);
    }
}

pub(super) fn push_indexed_db_transaction_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    operation: IndexedDbPendingTransactionOperation,
) -> Result<(), IndexedDbError> {
    let id = indexed_db_typed_state_id(scope, transaction).ok_or_else(|| {
        IndexedDbError::TransactionInactive("IndexedDB transaction was released".into())
    })?;
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    let mut table = table.borrow_mut();
    let transaction = table.transactions.get_mut(&id).ok_or_else(|| {
        IndexedDbError::TransactionInactive("IndexedDB transaction was released".into())
    })?;
    if transaction.finished || transaction.aborted {
        return Err(IndexedDbError::TransactionInactive(
            "IndexedDB transaction is no longer running".into(),
        ));
    }
    transaction
        .operations
        .try_reserve(1)
        .map_err(|error| IndexedDbError::Resource(error.to_string()))?;
    transaction.operations.push_back(operation);
    Ok(())
}

pub(super) fn pop_indexed_db_transaction_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) -> Option<IndexedDbPendingTransactionOperation> {
    let id = indexed_db_typed_state_id(scope, transaction)?;
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    let mut table = table.borrow_mut();
    let transaction = table.transactions.get_mut(&id)?;
    if !transaction.started
        || transaction.finished
        || transaction.aborted
        || transaction.operation_in_flight
    {
        return None;
    }
    let operation = transaction.operations.pop_front();
    transaction.operation_in_flight = operation.is_some();
    operation
}

pub(super) fn complete_indexed_db_transaction_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) {
    let Some(id) = indexed_db_typed_state_id(scope, transaction) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    if let Some(transaction) = table.borrow_mut().transactions.get_mut(&id) {
        transaction.operation_in_flight = false;
    }
}

pub(super) fn take_indexed_db_transaction_operations<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) -> std::collections::VecDeque<IndexedDbPendingTransactionOperation> {
    let Some(id) = indexed_db_typed_state_id(scope, transaction) else {
        return Default::default();
    };
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    let mut table = table.borrow_mut();
    table
        .transactions
        .get_mut(&id)
        .map(|transaction| std::mem::take(&mut transaction.operations))
        .unwrap_or_default()
}

pub(super) fn indexed_db_database_sql_client<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
) -> Option<moli_indexeddb::sqlite::service::DatabaseClient> {
    let id = indexed_db_typed_state_id(scope, database)?;
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    Some(table.borrow().databases.get(&id)?.backend.client.clone())
}

pub(super) fn indexed_db_database_sql_connection(
    scope: &mut v8::PinScope<'_, '_>,
    database: v8::Local<'_, v8::Object>,
) -> Option<moli_indexeddb::sqlite::service::DatabaseConnection> {
    let id = indexed_db_typed_state_id(scope, database)?;
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    table
        .borrow()
        .databases
        .get(&id)?
        .backend
        .connection
        .clone()
}

pub(super) fn attach_indexed_db_sql_connection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    connection: moli_indexeddb::sqlite::service::DatabaseConnection,
) -> Result<(), IndexedDbError> {
    let id = indexed_db_typed_state_id(scope, database)
        .ok_or_else(|| IndexedDbError::InvalidState("SQL database wrapper was released".into()))?;
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    let mut table = table.borrow_mut();
    let state = table
        .databases
        .get_mut(&id)
        .ok_or_else(|| IndexedDbError::InvalidState("SQL database wrapper was released".into()))?;
    let IndexedDbDatabaseBackend {
        client,
        connection: slot,
        forced_close,
    } = &mut state.backend;
    if state.handle != connection.handle()
        || client.id() != connection.client().id()
        || slot.is_some()
    {
        return Err(IndexedDbError::InvalidState(
            "SQL connection identity mismatch".into(),
        ));
    }
    *forced_close = Some(connection.close_signal());
    *slot = Some(connection.clone());
    drop(table);
    if let Some(host) = context_host_ptr_from_global_bridge(scope) {
        let owner = indexed_db_typed_execution_owner(scope, database)
            .and_then(IndexedDbExecutionOwner::execution_context)
            .expect("Page connection retains its exact Window owner");
        unsafe { &*host }.register_indexed_db_open_connection(owner, &connection);
    }
    Ok(())
}

pub(super) fn take_indexed_db_connection_request(
    scope: &mut v8::PinScope<'_, '_>,
    request: v8::Local<'_, v8::Object>,
) -> Option<moli_indexeddb::ConnectionRequestLease> {
    let id = indexed_db_typed_state_id(scope, request)?;
    let table = indexed_db_runtime_state_table_for_object(scope, request);
    table
        .borrow_mut()
        .requests
        .get_mut(&id)?
        .connection_request
        .take()
}

pub(super) fn take_indexed_db_delete_old_version(
    scope: &mut v8::PinScope<'_, '_>,
    request: v8::Local<'_, v8::Object>,
) -> Option<u64> {
    let id = indexed_db_typed_state_id(scope, request)?;
    let table = indexed_db_runtime_state_table_for_object(scope, request);
    table
        .borrow_mut()
        .requests
        .get_mut(&id)?
        .delete_old_version
        .take()
}

pub(super) fn take_indexed_db_version_change_completion(
    scope: &mut v8::PinScope<'_, '_>,
    task: v8::Local<'_, v8::Object>,
) -> Option<moli_indexeddb::sqlite::service::VersionChangeAcknowledgement> {
    let id = indexed_db_typed_task_id(scope, task)?;
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    table
        .borrow_mut()
        .version_change_tasks
        .get_mut(&id)?
        .completion
        .take()
}

pub(super) fn indexed_db_native_database_handle(
    scope: &mut v8::PinScope<'_, '_>,
    database: v8::Local<'_, v8::Object>,
) -> Option<DatabaseHandle> {
    let id = indexed_db_typed_state_id(scope, database)?;
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    table.borrow().databases.get(&id).map(|state| state.handle)
}

pub(super) fn indexed_db_native_transaction_handle(
    scope: &mut v8::PinScope<'_, '_>,
    transaction: v8::Local<'_, v8::Object>,
) -> Option<TransactionHandle> {
    let id = indexed_db_typed_state_id(scope, transaction)?;
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    table.borrow().transactions.get(&id)?.handle
}

pub(super) fn register_indexed_db_database_lifecycle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    handle: DatabaseHandle,
    storage_scope: IndexedDbStorageScope,
    client: moli_indexeddb::sqlite::service::DatabaseClient,
) {
    debug_assert!(storage_scope.has_explicit_partition_boundary());
    let Some(id) = indexed_db_typed_state_id(scope, database) else {
        return;
    };
    let backend = IndexedDbDatabaseBackend {
        client,
        connection: None,
        forced_close: None,
    };
    let mut state =
        IndexedDbDatabaseLifecycleState::new(scope, database, backend, handle, storage_scope);
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    let mirror = Rc::new(RefCell::new(IndexedDbDatabaseMirror::default()));
    state.mirror = Rc::downgrade(&mirror);
    table.borrow_mut().databases.insert(id, state);
    track_collectible_wrapper(scope, database, id, &table, mirror);
}

// Materialization has not published an open connection yet. Failed metadata
// admission must release all partially registered native state immediately.
pub(super) fn discard_indexed_db_database_lifecycle(
    scope: &mut v8::PinScope<'_, '_>,
    database: v8::Local<'_, v8::Object>,
) {
    let Some(id) = indexed_db_typed_state_id(scope, database) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    let mut table = table.borrow_mut();
    table.databases.remove(&id);
    table.wrappers.remove(&id);
    table.collectible_wrappers.remove(&id);
}

pub(super) fn register_indexed_db_cursor_lifecycle(
    scope: &mut v8::PinScope<'_, '_>,
    cursor: v8::Local<'_, v8::Object>,
    entries: Vec<CursorSnapshotEntry>,
    direction: CursorDirection,
    key_only: bool,
) {
    let Some(id) = indexed_db_typed_state_id(scope, cursor) else {
        return;
    };
    let position = (!entries.is_empty()).then_some(0);
    let state = IndexedDbCursorLifecycleState {
        entries: entries.into(),
        position,
        got_value: position.is_some(),
        direction,
        key_only,
        sql: None,
    };
    let table = indexed_db_runtime_state_table_for_object(scope, cursor);
    let state = Rc::new(RefCell::new(state));
    table.borrow_mut().cursors.insert(id, Rc::downgrade(&state));
    track_collectible_wrapper(scope, cursor, id, &table, state);
}

pub(super) fn indexed_db_cursor_state(
    scope: &mut v8::PinScope<'_, '_>,
    cursor: v8::Local<'_, v8::Object>,
) -> Option<IndexedDbCursorLifecycleState> {
    let state = cursor_native_state(scope, cursor)?;
    let snapshot = state.borrow().clone();
    Some(snapshot)
}

pub(super) fn set_indexed_db_cursor_position(
    scope: &mut v8::PinScope<'_, '_>,
    cursor: v8::Local<'_, v8::Object>,
    position: Option<usize>,
) -> Option<()> {
    let state = cursor_native_state(scope, cursor)?;
    let mut state = state.borrow_mut();
    if let Some(position) = position {
        state.entries.get(position)?;
    }
    state.position = position;
    state.got_value = position.is_some();
    Some(())
}

pub(super) fn begin_indexed_db_cursor_iteration(
    scope: &mut v8::PinScope<'_, '_>,
    cursor: v8::Local<'_, v8::Object>,
) -> Option<()> {
    let state = cursor_native_state(scope, cursor)?;
    // Keep the position and exposed values until the asynchronous iteration
    // settles, but reject further navigation and mutation immediately.
    state.borrow_mut().got_value = false;
    Some(())
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
    let state =
        IndexedDbObjectStoreLifecycleState::new(scope, store, transaction, database, metadata);
    let table = indexed_db_runtime_state_table_for_object(scope, store);
    let state = Rc::new(RefCell::new(state));
    table
        .borrow_mut()
        .object_stores
        .insert(id, Rc::downgrade(&state));
    track_collectible_wrapper(scope, store, id, &table, state);
    track_indexed_db_handle_table(scope, database, &table);
}

pub(super) fn register_indexed_db_index_lifecycle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: v8::Local<'s, v8::Object>,
    object_store: v8::Local<'s, v8::Object>,
    info: &IndexInfo,
) -> Option<()> {
    let id = indexed_db_typed_state_id(scope, index)?;
    let metadata = indexed_db_object_store_metadata(scope, object_store)?;
    let created_in_upgrade = metadata.index_created_in_upgrade(&info.name);
    let info = metadata.shared_index(&info.name)?;
    let database = indexed_db_object_store_database(scope, object_store);
    let table = indexed_db_runtime_state_table_for_object(scope, index);
    let state =
        IndexedDbIndexLifecycleState::new(scope, index, object_store, info, created_in_upgrade);
    let state = Rc::new(RefCell::new(state));
    table.borrow_mut().indexes.insert(id, Rc::downgrade(&state));
    track_collectible_wrapper(scope, index, id, &table, state);
    if let Some(database) = database {
        track_indexed_db_handle_table(scope, database, &table);
    }
    Some(())
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

#[cfg(test)]
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
    let resolver = payload.resolver.to_local(scope);
    let resolver = v8::Local::<v8::Object>::try_from(resolver).ok()?;
    let resolver = unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(resolver) };
    let value = payload.value.to_local(scope);
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
        IndexedDbTaskKind::RequestSuccess
            | IndexedDbTaskKind::RequestError
            | IndexedDbTaskKind::OpenSuccess
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
    let request = payload.request.to_local(scope);
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
    let request = payload.request.to_local(scope);
    let database = payload.database.to_local(scope);
    let transaction = payload.transaction.to_local(scope);
    Some((
        v8::Local::<v8::Object>::try_from(request).ok()?,
        v8::Local::<v8::Object>::try_from(database).ok()?,
        v8::Local::<v8::Object>::try_from(transaction).ok()?,
        payload.old_version,
        payload.new_version,
    ))
}

pub(super) fn register_indexed_db_version_change_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
    database: v8::Local<'s, v8::Object>,
    old_version: u64,
    new_version: Option<u64>,
) {
    let owner = indexed_db_typed_execution_owner(scope, database)
        .expect("versionchange task retains its database owner");
    let storage_scope = indexed_db_typed_storage_scope(scope, database);
    let id = register_indexed_db_task_with_owner(
        scope,
        task,
        IndexedDbTaskKind::VersionChange,
        owner,
        storage_scope,
    );
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    table.borrow_mut().version_change_tasks.insert(
        id,
        IndexedDbVersionChangeTaskPayload {
            completion: None,
            database: v8::Global::new(scope, database),
            old_version,
            new_version,
        },
    );
}

pub(super) fn indexed_db_version_change_task_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) -> Option<(v8::Local<'s, v8::Object>, u64, Option<u64>)> {
    let id = indexed_db_typed_task_id(scope, task)?;
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    let table = table.borrow();
    let payload = table.version_change_tasks.get(&id)?;
    Some((
        v8::Local::new(scope, &payload.database),
        payload.old_version,
        payload.new_version,
    ))
}

pub(super) fn register_indexed_db_object_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
    kind: IndexedDbTaskKind,
    target: v8::Local<'s, v8::Object>,
) {
    debug_assert!(matches!(
        kind,
        IndexedDbTaskKind::TransactionStart
            | IndexedDbTaskKind::TransactionCommit
            | IndexedDbTaskKind::TransactionAbort
            | IndexedDbTaskKind::TransactionOperationError
            | IndexedDbTaskKind::TransactionCancel
            | IndexedDbTaskKind::DatabaseForcedClose
    ));
    let owner = indexed_db_typed_execution_owner(scope, target)
        .expect("IDB task target should have typed owner state");
    let storage_scope = indexed_db_typed_storage_scope(scope, target);
    let id = register_indexed_db_task_with_owner(scope, task, kind, owner, storage_scope);
    let payload = IndexedDbObjectTaskPayload::new(scope, target);
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    table.borrow_mut().object_tasks.insert(id, payload);
}

pub(super) fn indexed_db_object_task_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let id = indexed_db_typed_task_id(scope, task)?;
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    let table = table.borrow();
    let payload = table.object_tasks.get(&id)?;
    let target = payload.target.to_local(scope);
    v8::Local::<v8::Object>::try_from(target).ok()
}

pub(super) fn register_indexed_db_transaction_error_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
    transaction: v8::Local<'s, v8::Object>,
    error: IndexedDbError,
) {
    let transaction_id = indexed_db_typed_state_id(scope, transaction)
        .expect("internal transaction errors retain their typed owner");
    register_indexed_db_object_task(
        scope,
        task,
        IndexedDbTaskKind::TransactionOperationError,
        transaction,
    );
    let id = indexed_db_typed_task_id(scope, task).expect("registered transaction error task");
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    table
        .borrow_mut()
        .object_tasks
        .get_mut(&id)
        .expect("registered error payload")
        .failure = Some((transaction_id, error));
}

pub(super) fn take_indexed_db_transaction_task_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) -> Option<IndexedDbError> {
    let id = indexed_db_typed_task_id(scope, task)?;
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    table
        .borrow_mut()
        .object_tasks
        .get_mut(&id)?
        .failure
        .take()
        .map(|(_, error)| error)
}

pub(super) fn unregister_indexed_db_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) {
    let Some(id) = indexed_db_typed_task_id(scope, task) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    unregister_indexed_db_task_record(&mut table.borrow_mut(), id);
}

pub(super) fn capture_indexed_db_task_retirement(
    scope: &mut v8::PinScope<'_, '_>,
    task: v8::Local<'_, v8::Object>,
) -> impl FnOnce() + 'static {
    let id = indexed_db_typed_task_id(scope, task);
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    move || {
        if let Some(id) = id {
            unregister_indexed_db_task_record(&mut table.borrow_mut(), id);
        }
    }
}

fn unregister_indexed_db_task_record(table: &mut IndexedDbRuntimeStateTable, id: IndexedDbTaskId) {
    table.tasks.remove(&id);
    table.factory_requests.remove(&id);
    if let Some(payload) = table.sql_tasks.remove(&id)
        && let Some(transaction) = payload.transaction
    {
        sql::cancel_sql_transaction(table, transaction);
    }
    table.databases_settle_tasks.remove(&id);
    table.request_dispatch_tasks.remove(&id);
    table.open_tasks.remove(&id);
    if let Some(payload) = table.object_tasks.remove(&id)
        && let Some((transaction, _)) = payload.failure
        && table
            .transactions
            .get(&transaction)
            .is_some_and(|state| state.sql_client.is_some())
    {
        // A discarded error task cannot leave its logical operation or native
        // transaction waiting forever. Normal dispatch takes the error first.
        sql::cancel_sql_transaction(table, transaction);
    }
    table.version_change_tasks.remove(&id);
}

fn indexed_db_database_mirror(
    scope: &mut v8::PinScope<'_, '_>,
    database: v8::Local<'_, v8::Object>,
) -> Option<Rc<RefCell<IndexedDbDatabaseMirror>>> {
    let id = indexed_db_typed_state_id(scope, database)?;
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    table.borrow().databases.get(&id)?.mirror.upgrade()
}

pub(super) fn replace_indexed_db_database_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    metadata: impl IntoIterator<Item = IndexedDbObjectStoreMetadata>,
    sql_schema: std::sync::Arc<moli_indexeddb::sqlite::StoreDefinitions>,
) -> Result<(), IndexedDbError> {
    indexed_db_database_sql_client(scope, database).ok_or_else(missing_mirror)?;
    let mirror = indexed_db_database_mirror(scope, database).ok_or_else(missing_mirror)?;
    let stores = metadata
        .into_iter()
        .map(|metadata| (metadata.info.name.clone(), metadata))
        .collect();
    let metadata = Rc::new(stores);
    let mut mirror = mirror.borrow_mut();
    mirror.metadata = Some(metadata);
    mirror.sql_schema = Some(std::sync::Arc::downgrade(&sql_schema));
    Ok(())
}

pub(super) fn indexed_db_database_store_names(
    scope: &mut v8::PinScope<'_, '_>,
    database: v8::Local<'_, v8::Object>,
) -> Vec<moli_indexeddb::IndexedDbName> {
    let mirror = indexed_db_database_mirror(scope, database).expect("database mirror");
    mirror
        .borrow()
        .metadata
        .as_ref()
        .expect("open database has schema")
        .keys()
        .cloned()
        .collect()
}

pub(super) fn indexed_db_database_has_sql_schema<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    schema: &std::sync::Arc<moli_indexeddb::sqlite::StoreDefinitions>,
) -> bool {
    indexed_db_database_mirror(scope, database).is_some_and(|mirror| {
        mirror
            .borrow()
            .sql_schema
            .as_ref()
            .is_some_and(|previous| previous.ptr_eq(&std::sync::Arc::downgrade(schema)))
    })
}

// The renderer candidate was built before each schema operation was accepted.
// After SQL COMMIT, promote it without allocating another native definition.
pub(super) fn promote_indexed_db_sql_schema<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    schema: &std::sync::Arc<moli_indexeddb::sqlite::StoreDefinitions>,
) {
    let Some(database) = indexed_db_transaction_database(scope, transaction) else {
        return;
    };
    let Some(mirror) = indexed_db_database_mirror(scope, database) else {
        return;
    };
    let mut mirror = mirror.borrow_mut();
    if mirror.upgrade_metadata.is_none() {
        return;
    }
    if let Some(metadata) = &mirror.metadata {
        for store in metadata.values() {
            store.created_in_upgrade.set(false);
            store.created_indexes.borrow_mut().take();
        }
    }
    mirror.sql_schema = Some(std::sync::Arc::downgrade(schema));
}

pub(super) fn indexed_db_database_store_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    store_name: &moli_indexeddb::IndexedDbName,
) -> Option<IndexedDbObjectStoreMetadata> {
    let mirror = indexed_db_database_mirror(scope, database)?;
    mirror.borrow().metadata.as_ref()?.get(store_name).cloned()
}

pub(super) fn set_indexed_db_database_store_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    mut metadata: IndexedDbObjectStoreMetadata,
) -> Result<(), IndexedDbError> {
    indexed_db_database_sql_client(scope, database).ok_or_else(missing_mirror)?;
    let mirror = indexed_db_database_mirror(scope, database).ok_or_else(missing_mirror)?;
    let mut mirror = mirror.borrow_mut();
    metadata.mark_created_in_upgrade(mirror.upgrade_metadata.is_some());
    let current = mirror.metadata.as_ref().ok_or_else(missing_mirror)?;
    let candidate = edit_shared(current, |map| {
        map.insert(metadata.info.name.clone(), metadata);
    });
    mirror.metadata = Some(candidate);
    Ok(())
}

pub(super) fn remove_indexed_db_database_store_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    store_name: &moli_indexeddb::IndexedDbName,
) -> Result<(), IndexedDbError> {
    indexed_db_database_sql_client(scope, database).ok_or_else(missing_mirror)?;
    let mirror = indexed_db_database_mirror(scope, database).ok_or_else(missing_mirror)?;
    let mut mirror = mirror.borrow_mut();
    let current = mirror.metadata.as_ref().ok_or_else(missing_mirror)?;
    let candidate = edit_shared(current, |map| {
        map.remove(store_name);
        if map.is_empty() {
            *map = BTreeMap::new();
        }
    });
    mirror.metadata = Some(candidate);
    Ok(())
}

pub(super) fn set_indexed_db_database_index_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    store_name: &moli_indexeddb::IndexedDbName,
    info: &IndexInfo,
) -> Result<(), IndexedDbError> {
    indexed_db_database_sql_client(scope, database).ok_or_else(missing_mirror)?;
    let mirror = indexed_db_database_mirror(scope, database).ok_or_else(missing_mirror)?;
    let mut mirror = mirror.borrow_mut();
    let current = mirror.metadata.as_ref().ok_or_else(missing_mirror)?;
    let store = current
        .get(store_name)
        .ok_or_else(missing_mirror)?
        .with_index(info);
    let candidate = edit_shared(current, |map| {
        *map.get_mut(store_name).expect("store in current candidate") = store;
    });
    mirror.metadata = Some(candidate);
    Ok(())
}

pub(super) fn remove_indexed_db_database_index_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    store_name: &moli_indexeddb::IndexedDbName,
    index_name: &moli_indexeddb::IndexedDbName,
) -> Result<(), IndexedDbError> {
    indexed_db_database_sql_client(scope, database).ok_or_else(missing_mirror)?;
    let mirror = indexed_db_database_mirror(scope, database).ok_or_else(missing_mirror)?;
    let mut mirror = mirror.borrow_mut();
    let current = mirror.metadata.as_ref().ok_or_else(missing_mirror)?;
    let store = current
        .get(store_name)
        .ok_or_else(missing_mirror)?
        .without_index(index_name);
    let candidate = edit_shared(current, |map| {
        *map.get_mut(store_name).expect("store in current candidate") = store;
    });
    mirror.metadata = Some(candidate);
    Ok(())
}

pub(super) fn indexed_db_object_store_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
) -> Option<IndexedDbObjectStoreMetadata> {
    Some(
        object_store_native_state(scope, store)?
            .borrow()
            .metadata
            .clone(),
    )
}
pub(super) fn indexed_db_object_store_transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    object_store_native_state(scope, store)?
        .borrow()
        .transaction
        .to_local(scope)
}
pub(super) fn indexed_db_object_store_database<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    object_store_native_state(scope, store)?
        .borrow()
        .database
        .to_local(scope)
}
pub(super) fn indexed_db_object_store_name(
    scope: &mut v8::PinScope<'_, '_>,
    store: v8::Local<'_, v8::Object>,
) -> Option<moli_indexeddb::IndexedDbName> {
    Some(
        object_store_native_state(scope, store)?
            .borrow()
            .name()
            .to_owned(),
    )
}
pub(super) fn indexed_db_index_object_store<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    index_native_state(scope, index)?
        .borrow()
        .object_store
        .to_local(scope)
}
pub(super) fn indexed_db_index_info(
    scope: &mut v8::PinScope<'_, '_>,
    index: v8::Local<'_, v8::Object>,
) -> Option<IndexInfo> {
    Some(
        index_native_state(scope, index)?
            .borrow()
            .info
            .as_ref()
            .clone(),
    )
}
pub(super) fn current_indexed_db_execution_owner(
    scope: &mut v8::PinScope<'_, '_>,
) -> IndexedDbExecutionOwner {
    if let Some(host_ptr) = crate::util::context_host_ptr_from_global_bridge(scope) {
        let host = unsafe { &*host_ptr };
        let execution_context = host
            .window_execution_context_identity_for_v8_context(scope, scope.get_current_context());
        if let Some(execution_context) = execution_context {
            return IndexedDbExecutionOwner::Window(execution_context);
        }
    }

    IndexedDbExecutionOwner::PendingWindow(inferred_indexed_db_dispatch_scope(scope))
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
    if table.borrow().requests.contains_key(&id) && is_request_value_slot(key) {
        return request_value(scope, object, key);
    }
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
    if let Some(store) = table
        .object_stores
        .get(&id)
        .and_then(std::rc::Weak::upgrade)
    {
        return indexed_db_typed_object_store_slot_value(scope, &store.borrow(), key);
    }
    if let Some(index) = table.indexes.get(&id).and_then(std::rc::Weak::upgrade) {
        return indexed_db_typed_index_slot_value(scope, &index.borrow(), key);
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
    if table.borrow().requests.contains_key(&id) && is_request_value_slot(key) {
        set_private_value(scope, object, key, value);
        return true;
    }
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
    if let Some(store) = table
        .object_stores
        .get(&id)
        .and_then(std::rc::Weak::upgrade)
    {
        return set_indexed_db_typed_object_store_slot_value(
            scope,
            &mut store.borrow_mut(),
            key,
            value,
        );
    }
    if let Some(index) = table.indexes.get(&id).and_then(std::rc::Weak::upgrade) {
        return set_indexed_db_typed_index_slot_value(&mut index.borrow_mut(), key, value);
    }
    false
}

fn indexed_db_typed_request_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: &IndexedDbRequestLifecycleState,
    key: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    match key {
        INDEXED_DB_REQUEST_READY_STATE_SLOT => {
            v8_string(scope, &request.ready_state).map(Into::into)
        }
        INDEXED_DB_REQUEST_BLOCKED_DISPATCHED_SLOT => {
            Some(v8::Boolean::new(scope, request.blocked_dispatched).into())
        }
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
        INDEXED_DB_REQUEST_READY_STATE_SLOT => {
            request.ready_state = value
                .to_string(scope)
                .map(|value| value.to_rust_string_lossy(scope))
                .unwrap_or_default();
            true
        }
        INDEXED_DB_REQUEST_BLOCKED_DISPATCHED_SLOT => {
            request.blocked_dispatched = value.boolean_value(scope);
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
        INDEXED_DB_TRANSACTION_COMMITTING_SLOT => {
            Some(v8::Boolean::new(scope, transaction.committing).into())
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
        INDEXED_DB_TRANSACTION_COMMITTING_SLOT => {
            transaction.committing = value.boolean_value(scope);
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
        _ => false,
    }
}

fn create_database_metadata_object_from_typed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    metadata: &BTreeMap<moli_indexeddb::IndexedDbName, IndexedDbObjectStoreMetadata>,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = new_null_prototype_object(scope);
    for (store_name, store) in metadata {
        let descriptor = create_object_store_descriptor_from_typed(scope, store)?;
        set_indexed_db_internal_name_property(scope, object, store_name, descriptor.into());
    }
    Some(object)
}

fn create_object_store_descriptor_from_typed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    metadata: &IndexedDbObjectStoreMetadata,
) -> Option<v8::Local<'s, v8::Object>> {
    create_object_store_descriptor_object(scope, metadata)
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
        INDEXED_DB_DATABASE_STORAGE_KEY_SLOT => {
            v8_string(scope, database.storage_scope.storage_key()).map(Into::into)
        }
        INDEXED_DB_DATABASE_CLOSED_SLOT => Some(v8::Boolean::new(scope, database.closed).into()),
        INDEXED_DB_DATABASE_METADATA_SLOT => database.mirror.upgrade().and_then(|mirror| {
            let mirror = mirror.borrow();
            create_database_metadata_object_from_typed(scope, mirror.metadata.as_ref()?)
                .map(Into::into)
        }),
        INDEXED_DB_DATABASE_UPGRADE_TRANSACTION_SLOT => database
            .upgrade_transaction
            .as_ref()
            .map(|value| value.to_local(scope)),
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
                if let Some(mirror) = database.mirror.upgrade() {
                    mirror.borrow_mut().upgrade_metadata = None;
                }
                None
            } else {
                Some(RealmValueHandle::new(scope, value))
            };
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
        INDEXED_DB_OBJECT_STORE_NAME_SLOT => {
            v8::String::new_from_two_byte(scope, store.name().as_utf16(), v8::NewStringType::Normal)
                .map(Into::into)
        }
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
            store.name_override = Some(
                value
                    .to_string(scope)
                    .map(|value| {
                        moli_indexeddb::IndexedDbName::from_utf16(
                            crate::util::v8_string_to_u16_string(scope, value).into_vec(),
                        )
                    })
                    .unwrap_or_default(),
            );
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

pub(super) fn set_indexed_db_transaction_start_request(
    scope: &mut v8::PinScope<'_, '_>,
    transaction: v8::Local<'_, v8::Object>,
    request: moli_indexeddb::TransactionRequestLease,
) {
    let id = indexed_db_typed_state_id(scope, transaction).expect("transaction id");
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    let mut table = table.borrow_mut();
    let state = table.transactions.get_mut(&id).expect("transaction state");
    assert!(state.start_request.is_none(), "transaction scheduled twice");
    state.start_request = Some(request);
}

pub(super) fn indexed_db_transaction_start_request(
    scope: &mut v8::PinScope<'_, '_>,
    transaction: v8::Local<'_, v8::Object>,
) -> Option<moli_indexeddb::TransactionRequestHandle> {
    let id = indexed_db_typed_state_id(scope, transaction)?;
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    Some(
        table
            .borrow()
            .transactions
            .get(&id)?
            .start_request
            .as_ref()?
            .handle()
            .clone(),
    )
}

pub(super) fn finish_indexed_db_transaction_start_request(
    scope: &mut v8::PinScope<'_, '_>,
    transaction: v8::Local<'_, v8::Object>,
) {
    let Some(id) = indexed_db_typed_state_id(scope, transaction) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    let request = table
        .borrow_mut()
        .transactions
        .get_mut(&id)
        .and_then(|state| state.start_request.take());
    // Releasing admission wakes other event loops after native commit/rollback
    // and after releasing the local state borrow.
    drop(request);
}

impl IndexedDbRuntimeStateTable {
    fn retire_connections(&mut self) {
        // Dropping each emergency guard schedules native rollback before its
        // admission is released. Wakes only publish owner task tickets.
        for transaction in self.transactions.values_mut() {
            drop(transaction.sql_rollback.take());
            drop(transaction.start_request.take());
        }
        for database in self.databases.values_mut() {
            if let Some(connection) = database.backend.connection.take() {
                let _ = connection.retire();
            }
        }
        self.sql_tasks.clear();
        self.factory_requests.clear();
        self.version_change_tasks.clear();
        for request in self.requests.values_mut() {
            drop(request.connection_request.take());
        }
    }
}
impl Drop for IndexedDbRuntimeStateTable {
    fn drop(&mut self) {
        self.retire_connections();
    }
}
pub(crate) fn retire_indexed_db_context(context: v8::Local<'_, v8::Context>) {
    if let Some(table) = context.get_slot::<RefCell<IndexedDbRuntimeStateTable>>() {
        table.borrow_mut().retire_connections();
    }
}
pub(super) fn indexed_db_database_has_unfinished_transactions(
    scope: &mut v8::PinScope<'_, '_>,
    database: v8::Local<'_, v8::Object>,
) -> bool {
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    table.borrow().transactions.values().any(|state| {
        !state.finished
            && state
                .database
                .as_ref()
                .is_some_and(|value| value.to_local(scope).strict_equals(database.into()))
    })
}

pub(super) fn indexed_db_transaction_database<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let id = indexed_db_typed_state_id(scope, transaction)?;
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    let value = table
        .borrow()
        .transactions
        .get(&id)?
        .database
        .as_ref()?
        .to_local(scope);
    v8::Local::<v8::Object>::try_from(value).ok()
}

pub(super) fn mark_indexed_db_transaction_cancellation_scheduled(
    scope: &mut v8::PinScope<'_, '_>,
    transaction: v8::Local<'_, v8::Object>,
) -> bool {
    let Some(id) = indexed_db_typed_state_id(scope, transaction) else {
        return false;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    let mut table = table.borrow_mut();
    let Some(state) = table.transactions.get_mut(&id) else {
        return false;
    };
    if state.finished || state.cancellation_scheduled {
        return false;
    }
    state.cancellation_scheduled = true;
    true
}

pub(super) fn indexed_db_database_is_forced_closed(
    scope: &mut v8::PinScope<'_, '_>,
    database: v8::Local<'_, v8::Object>,
) -> bool {
    let Some(id) = indexed_db_typed_state_id(scope, database) else {
        return false;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    table
        .borrow()
        .databases
        .get(&id)
        .is_some_and(|state| state.backend.is_forced_closed())
}

pub(super) fn indexed_db_forced_closed_databases<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Vec<v8::Local<'s, v8::Object>> {
    let table = ensure_indexed_db_runtime_state_table(scope);
    table
        .borrow()
        .databases
        .values()
        .filter(|state| state.backend.is_forced_closed() && !state.forced_close_scheduled)
        .filter_map(|state| state.wrapper.to_local(scope))
        .collect()
}

pub(super) fn indexed_db_database_has_pending_transaction_events(
    scope: &mut v8::PinScope<'_, '_>,
    database: v8::Local<'_, v8::Object>,
) -> bool {
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    // A finished transaction retains this edge through its terminal event.
    table.borrow().transactions.values().any(|state| {
        state
            .database
            .as_ref()
            .is_some_and(|value| value.to_local(scope).strict_equals(database.into()))
    })
}

pub(super) fn mark_indexed_db_database_forced_close_scheduled(
    scope: &mut v8::PinScope<'_, '_>,
    database: v8::Local<'_, v8::Object>,
) -> bool {
    let Some(id) = indexed_db_typed_state_id(scope, database) else {
        return false;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    let mut table = table.borrow_mut();
    let Some(state) = table.databases.get_mut(&id) else {
        return false;
    };
    if state.forced_close_scheduled || !state.backend.is_forced_closed() {
        return false;
    }
    state.closed = true;
    state.forced_close_scheduled = true;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "IndexedDB runtime task id space exhausted")]
    fn runtime_task_ids_never_saturate() {
        let mut table = IndexedDbRuntimeStateTable::default();
        table.next_id = u64::MAX;

        let _ = table.upsert_task(
            None,
            IndexedDbTaskState::new(
                IndexedDbTaskKind::FactoryRequest,
                IndexedDbExecutionOwner::without_execution_context(
                    crate::native_bridge::OwnerDispatchScope::Top,
                ),
                None,
            ),
        );
    }

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
                IndexedDbTaskKind::FactoryRequest,
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
        let mut metadata = IndexedDbObjectStoreMetadata::from_info(&ObjectStoreInfo {
            name: "posts".into(),
            key_path: None,
            auto_increment: false,
            index_names: Vec::new(),
        });
        let index = IndexInfo {
            name: "by-tag".into(),
            key_path: KeyPath::String("tag".to_owned()),
            unique: true,
            multi_entry: false,
        };

        metadata = metadata.with_index(&index);

        assert_eq!(metadata.info().index_names, ["by-tag"]);
        assert_eq!(metadata.index(&"by-tag".into()), Some(&index));
        assert_eq!(
            metadata.indexes_in_name_order(),
            std::slice::from_ref(&index)
        );

        metadata = metadata.without_index(&"by-tag".into());

        assert!(metadata.info().index_names.is_empty());
        assert!(metadata.index(&"by-tag".into()).is_none());
        assert!(metadata.indexes_in_name_order().is_empty());
    }
}

#[cfg(test)]
mod collection_tests;

#[cfg(test)]
mod metadata_sharing_tests;
#[cfg(test)]
pub(crate) use metadata_sharing_tests::{
    IndexedDbMetadataSharingProbe, indexed_db_metadata_sharing_probe_for_test,
};

#[cfg(test)]
pub(crate) fn indexed_db_native_cursor_count_for_test(scope: &mut v8::PinScope<'_, '_>) -> usize {
    let table = ensure_indexed_db_runtime_state_table(scope);
    table.borrow().cursors.len()
}
