//! Open/delete algorithms use the native connection FIFO and the same owner
//! completion transport as transaction commands. No SQL runs on this thread.

use super::*;
use moli_indexeddb::sqlite::service::{
    Command, CommandResult, DatabaseClient, DatabaseConnection, ServiceClient,
};
use moli_indexeddb::{ConnectionRequestLease, VersionChangeBatch};
use std::sync::Arc;

#[derive(Clone, Copy)]
pub(in crate::context_bootstrap::indexed_db) enum FactoryOperation {
    Open(Option<u64>),
    Delete,
}

enum Stage {
    Queued,
    Waiting,
    Ready,
    Notifying {
        old_version: u64,
        new_version: Option<u64>,
        batch: VersionChangeBatch,
        blocked: bool,
    },
    Failed(IndexedDbError),
}

pub(super) struct FactoryRequest {
    // Drop native cleanup before releasing the FIFO head, including when the
    // accepting realm disappears while its upgrade BEGIN is still in flight.
    rollback: Option<sql::SqlTransactionRollback>,
    connection: Option<DatabaseConnection>,
    lease: Option<ConnectionRequestLease>,
    request: RealmValueHandle,
    service: ServiceClient,
    storage_scope: IndexedDbStorageScope,
    name: String,
    operation: FactoryOperation,
    client: Option<DatabaseClient>,
    stage: Stage,
    wake: Arc<dyn Fn() -> bool + Send + Sync>,
}

#[cfg(test)]
impl FactoryRequest {
    pub(super) fn has_native_close_receipt_for_test(&self) -> bool {
        // Unregistration can precede the notification acknowledgement and its
        // owner wake. A requesting realm must not be considered idle in between.
        matches!(self.stage, Stage::Notifying { .. })
            && self
                .client
                .as_ref()
                .is_some_and(|client| !client.has_open_connections())
    }

    pub(super) fn waits_for_close_for_test(&self, client: &DatabaseClient) -> bool {
        matches!(self.stage, Stage::Notifying { .. })
            && self.client.as_ref().is_some_and(|candidate| {
                candidate.id() == client.id()
                    && candidate.identity().generation == client.identity().generation
            })
    }
}

pub(in crate::context_bootstrap::indexed_db) fn enqueue_sql_factory_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    service: ServiceClient,
    storage_scope: IndexedDbStorageScope,
    name: String,
    operation: FactoryOperation,
) -> Result<(), IndexedDbError> {
    let owner = indexed_db_typed_execution_owner(scope, request)
        .ok_or_else(|| IndexedDbError::InvalidState("open/delete request lost its owner".into()))?;
    let task = v8::Object::new(scope);
    let id = register_indexed_db_task_with_owner(
        scope,
        task,
        IndexedDbTaskKind::FactoryRequest,
        owner,
        Some(storage_scope.clone()),
    );
    let wake = super::super::runtime::indexed_db_sql_completion_wake(scope, owner, id);
    let queued_wake = wake.clone();
    let lease = match service.queue_connection_request(
        storage_scope.storage_key().as_bytes(),
        &name,
        Arc::new(move || {
            queued_wake();
        }),
    ) {
        Ok(lease) => lease,
        Err(error) => {
            unregister_indexed_db_task(scope, task);
            return Err(error);
        }
    };
    let payload = FactoryRequest {
        rollback: None,
        connection: None,
        lease: Some(lease),
        request: RealmValueHandle::new(scope, request.into()),
        service,
        storage_scope,
        name,
        operation,
        client: None,
        stage: Stage::Queued,
        wake,
    };
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    table
        .borrow_mut()
        .factory_requests
        .insert(id, Rc::new(RefCell::new(payload)));
    enqueue_indexed_db_task(scope, task);
    Ok(())
}

pub(in crate::context_bootstrap::indexed_db) fn factory_task_is_ready(
    table: &IndexedDbRuntimeStateTable,
    id: IndexedDbTaskId,
) -> bool {
    let Some(payload) = table.factory_requests.get(&id) else {
        return false;
    };
    let payload = payload.borrow();
    let Some(lease) = &payload.lease else {
        return false;
    };
    if !lease.handle().is_active() || payload.service.is_closed() {
        return true;
    }
    if !lease.handle().is_head() {
        return false;
    }
    match &payload.stage {
        Stage::Queued | Stage::Ready | Stage::Failed(_) => true,
        Stage::Waiting => false,
        Stage::Notifying { batch, blocked, .. } => match batch.blocked_event_required() {
            None => false,
            Some(true) if !blocked => true,
            Some(_) => payload
                .client
                .as_ref()
                .is_some_and(|client| !client.has_open_connections()),
        },
    }
}

pub(in crate::context_bootstrap::indexed_db) fn retain_pending_factory_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(id) = indexed_db_typed_task_id(scope, task) else {
        return false;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    let pending = table.borrow().factory_requests.contains_key(&id);
    if pending {
        // Retain in the existing task queue without repeatedly scheduling work
        // whose native head/notification/completion condition is not ready.
        push_object_to_indexed_db_runtime_array(scope, IndexedDbRuntimeArray::TaskQueue, task);
    }
    pending
}

pub(in crate::context_bootstrap::indexed_db) fn flush_factory_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) {
    let Some(id) = indexed_db_typed_task_id(scope, task) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    let Some(payload) = table.borrow().factory_requests.get(&id).cloned() else {
        return;
    };
    let Ok(request) = v8::Local::<v8::Object>::try_from(payload.borrow().request.to_local(scope))
    else {
        table.borrow_mut().factory_requests.remove(&id);
        return;
    };
    if payload.borrow().service.is_closed()
        || payload
            .borrow()
            .lease
            .as_ref()
            .is_none_or(|lease| !lease.handle().is_active())
    {
        fail(
            scope,
            request,
            &payload,
            &table,
            id,
            IndexedDbError::InvalidState("IndexedDB storage is closed".into()),
        );
        return;
    }
    let storage_scope = payload.borrow().storage_scope.clone();
    if let Err(error) = validate_storage_bucket_scope(scope, &storage_scope) {
        fail(scope, request, &payload, &table, id, error);
        return;
    }
    let stage = std::mem::replace(&mut payload.borrow_mut().stage, Stage::Waiting);
    let result = match stage {
        Stage::Queued => resolve(scope, request, &payload),
        Stage::Ready => ready(scope, request, &payload, &table, id),
        Stage::Notifying {
            old_version,
            new_version,
            batch,
            blocked,
        } => {
            if batch.blocked_event_required() == Some(true) && !blocked {
                payload.borrow_mut().stage = Stage::Notifying {
                    old_version,
                    new_version,
                    batch,
                    blocked: true,
                };
                let _ = dispatch_version_change_event(
                    scope,
                    request,
                    "blocked",
                    old_version,
                    new_version,
                );
                // A second task follows the event's microtask checkpoint even
                // if all old connections have already closed in the meantime.
                (payload.borrow().wake)();
                Ok(())
            } else {
                finish_wait(
                    scope,
                    request,
                    &payload,
                    &table,
                    id,
                    old_version,
                    new_version,
                )
            }
        }
        Stage::Failed(error) => Err(error),
        Stage::Waiting => Ok(()),
    };
    if let Err(error) = result {
        fail(scope, request, &payload, &table, id, error);
    }
}

/// A discarded SQL completion must not strand its factory algorithm in Waiting.
/// This guard owns no V8 state and only wakes the accepting native task route.
struct AwaitCompletion {
    payload: std::rc::Weak<RefCell<FactoryRequest>>,
    armed: bool,
}

impl AwaitCompletion {
    fn new(payload: &Rc<RefCell<FactoryRequest>>) -> Self {
        Self {
            payload: Rc::downgrade(payload),
            armed: true,
        }
    }
    fn accept(mut self) -> Option<Rc<RefCell<FactoryRequest>>> {
        self.armed = false;
        self.payload.upgrade()
    }
}

impl Drop for AwaitCompletion {
    fn drop(&mut self) {
        if self.armed
            && let Some(payload) = self.payload.upgrade()
        {
            let wake = {
                let mut payload = payload.borrow_mut();
                payload.stage = Stage::Failed(IndexedDbError::InvalidState(
                    "IndexedDB factory completion was discarded".into(),
                ));
                payload.wake.clone()
            };
            wake();
        }
    }
}

fn resolve<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    payload: &Rc<RefCell<FactoryRequest>>,
) -> Result<(), IndexedDbError> {
    let (service, owner, name) = {
        let state = payload.borrow();
        // The Service/root is partition-local. Persist the full serialized
        // storage key (including top-level site and named bucket ID), never
        // an origin projection or ephemeral browser-context/realm identifiers.
        (
            state.service.clone(),
            state.storage_scope.storage_key().as_bytes().to_vec(),
            state.name.clone(),
        )
    };
    let target = Rc::downgrade(payload);
    let awaiting = AwaitCompletion::new(payload);
    sql::submit_indexed_db_sql_completion_task(
        scope,
        request,
        move |completion| {
            let client = service.resolve(owner, name, completion)?;
            if let Some(payload) = target.upgrade() {
                payload.borrow_mut().client = Some(client.clone());
            }
            Ok(client)
        },
        move |_, _, reply| {
            let Some(payload) = awaiting.accept() else {
                return;
            };
            let mut state = payload.borrow_mut();
            state.stage = match reply.result {
                Ok(CommandResult::Metadata(_)) => Stage::Ready,
                Ok(_) => Stage::Failed(IndexedDbError::Corruption(
                    "invalid factory resolve result".into(),
                )),
                Err(error) => Stage::Failed(error),
            };
            (state.wake)();
        },
    )
}

fn ready<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    payload: &Rc<RefCell<FactoryRequest>>,
    table: &Rc<RefCell<IndexedDbRuntimeStateTable>>,
    id: IndexedDbTaskId,
) -> Result<(), IndexedDbError> {
    let (client, operation, wake) = {
        let state = payload.borrow();
        (
            state.client.clone().expect("resolved SQL factory"),
            state.operation,
            state.wake.clone(),
        )
    };
    let old_version = client
        .committed()
        .ok_or_else(|| IndexedDbError::InvalidState("database metadata is unavailable".into()))?
        .version;
    let new_version = match operation {
        FactoryOperation::Open(version) => Some(version.unwrap_or(old_version.max(1))),
        FactoryOperation::Delete => None,
    };
    if let Some(version) = new_version {
        if version < old_version {
            return Err(IndexedDbError::Version(
                "The requested version is lower than the existing version".into(),
            ));
        }
        if version == old_version {
            let connection = create_connection(scope, request, &client)?;
            payload.borrow_mut().connection = Some(connection);
            materialize(scope, request, payload, old_version, version)?;
            table.borrow_mut().factory_requests.remove(&id);
            return Ok(());
        }
    }
    let batch = client.begin_version_change(
        old_version,
        new_version,
        Arc::new(move || {
            wake();
        }),
    );
    payload.borrow_mut().stage = Stage::Notifying {
        old_version,
        new_version,
        batch,
        blocked: false,
    };
    Ok(())
}

fn create_connection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    client: &DatabaseClient,
) -> Result<DatabaseConnection, IndexedDbError> {
    let owner = indexed_db_typed_execution_owner(scope, request)
        .ok_or_else(|| IndexedDbError::InvalidState("factory request lost its owner".into()))?;
    client.open_connection(super::super::runtime::indexed_db_transaction_start_wake(
        scope, owner,
    ))
}

fn finish_wait<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    payload: &Rc<RefCell<FactoryRequest>>,
    table: &Rc<RefCell<IndexedDbRuntimeStateTable>>,
    id: IndexedDbTaskId,
    old_version: u64,
    new_version: Option<u64>,
) -> Result<(), IndexedDbError> {
    let client = payload
        .borrow()
        .client
        .clone()
        .expect("resolved SQL factory");
    if let Some(version) = new_version {
        let transaction = client.allocate_transaction_handle()?;
        let rollback = sql::SqlTransactionRollback::new(client.clone(), transaction);
        let connection = create_connection(scope, request, &client)?;
        {
            let mut state = payload.borrow_mut();
            state.rollback = Some(rollback);
            state.connection = Some(connection.clone());
        }
        let awaiting = AwaitCompletion::new(payload);
        let table = Rc::downgrade(table);
        sql::submit_indexed_db_sql_completion_task(
            scope,
            request,
            move |completion| {
                connection.begin_upgrade(transaction, version, completion)?;
                Ok(client)
            },
            move |scope, request, reply| {
                let Some(payload) = awaiting.accept() else {
                    return;
                };
                let Some(table) = table.upgrade() else {
                    return;
                };
                let result = reply.result.and_then(|_| {
                    if reply.active_transaction != Some(transaction) {
                        return Err(IndexedDbError::TransactionInactive(
                            "upgrade SQL owner was lost".into(),
                        ));
                    }
                    materialize(scope, request, &payload, old_version, version)
                });
                match result {
                    Ok(()) => {
                        table.borrow_mut().factory_requests.remove(&id);
                    }
                    Err(error) => fail(scope, request, &payload, &table, id, error),
                }
            },
        )
    } else {
        let awaiting = AwaitCompletion::new(payload);
        let table = Rc::downgrade(table);
        sql::submit_indexed_db_sql_completion_task(
            scope,
            request,
            move |completion| {
                client.submit(Command::DeleteDatabase, completion)?;
                Ok(client)
            },
            move |scope, request, reply| {
                let Some(payload) = awaiting.accept() else {
                    return;
                };
                let Some(table) = table.upgrade() else {
                    return;
                };
                match reply.result {
                    Ok(_) => {
                        retain_request_lease(scope, request, &payload);
                        let request_id = indexed_db_typed_state_id(scope, request)
                            .expect("open/delete request state");
                        table
                            .borrow_mut()
                            .requests
                            .get_mut(&request_id)
                            .expect("open/delete lifecycle")
                            .delete_old_version = Some(old_version);
                        store_request_success(scope, request, v8::undefined(scope).into());
                        table.borrow_mut().factory_requests.remove(&id);
                    }
                    Err(error) => fail(scope, request, &payload, &table, id, error),
                }
            },
        )
    }
}

fn retain_request_lease(
    scope: &mut v8::PinScope<'_, '_>,
    request: v8::Local<'_, v8::Object>,
    payload: &Rc<RefCell<FactoryRequest>>,
) {
    let Some(id) = indexed_db_typed_state_id(scope, request) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, request);
    if let Some(state) = table.borrow_mut().requests.get_mut(&id) {
        state.connection_request = payload.borrow_mut().lease.take();
    }
}

fn fail<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    payload: &Rc<RefCell<FactoryRequest>>,
    table: &Rc<RefCell<IndexedDbRuntimeStateTable>>,
    id: IndexedDbTaskId,
    error: IndexedDbError,
) {
    if scope.is_execution_terminating() {
        if let Some(connection) = &payload.borrow().connection {
            let _ = connection.retire();
        }
        table.borrow_mut().factory_requests.remove(&id);
        return;
    }
    retain_request_lease(scope, request, payload);
    table.borrow_mut().factory_requests.remove(&id);
    let error = request_error_object(scope, &error);
    store_request_error(scope, request, error);
}

fn materialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    payload: &Rc<RefCell<FactoryRequest>>,
    old_version: u64,
    new_version: u64,
) -> Result<(), IndexedDbError> {
    let (connection, storage_scope) = {
        let state = payload.borrow();
        (
            state.connection.clone().expect("accepted SQL connection"),
            state.storage_scope.clone(),
        )
    };
    let client = connection.client();
    let metadata = client.committed().ok_or_else(|| {
        IndexedDbError::InvalidState("SQL database metadata is unavailable".into())
    })?;
    let owner = indexed_db_typed_execution_owner(scope, request)
        .ok_or_else(|| IndexedDbError::InvalidState("factory request lost its owner".into()))?;
    let database = create_database_object(
        scope,
        storage_scope,
        owner,
        connection.handle(),
        &metadata.name,
        new_version,
        client.clone(),
    )?;
    let cleanup_connection = connection.clone();
    let result = (|| {
        attach_indexed_db_sql_connection(scope, database, connection)?;
        if new_version == old_version {
            retain_request_lease(scope, request, payload);
            store_request_success(scope, request, database.into());
            return Ok(());
        }
        let rollback = payload
            .borrow_mut()
            .rollback
            .take()
            .expect("armed upgrade rollback");
        let mut store_names = indexed_db_database_store_names(scope, database);
        store_names.sort_unstable_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
        let transaction = create_transaction_object(
            scope,
            database,
            None,
            TransactionMode::VersionChange,
            &store_names,
        )
        .ok_or_else(|| {
            IndexedDbError::InvalidState("unable to create upgrade transaction".into())
        })?;
        sql::configure_sql_upgrade(scope, transaction, rollback)?;
        set_indexed_db_slot_value(
            scope,
            transaction,
            INDEXED_DB_TRANSACTION_STARTED_SLOT,
            v8::Boolean::new(scope, true).into(),
        );
        set_indexed_db_slot_value(
            scope,
            database,
            INDEXED_DB_DATABASE_UPGRADE_TRANSACTION_SLOT,
            transaction.into(),
        );
        save_indexed_db_upgrade_metadata(scope, database, old_version);
        retain_request_lease(scope, request, payload);
        enqueue_open_task(
            scope,
            request,
            database,
            transaction,
            old_version,
            new_version,
        );
        Ok(())
    })();
    if result.is_err() {
        let _ = cleanup_connection.retire();
        if scope.is_execution_terminating() {
            return result;
        }
        close_indexed_db_database_connection(scope, database);
        let handle = info_handle(scope, database);
        if let Some(host) = context_host_ptr_from_global_bridge(scope) {
            unsafe { &*host }.unregister_indexed_db_open_connection(handle);
        }
        discard_indexed_db_database_lifecycle(scope, database);
    }
    result
}

fn info_handle(
    scope: &mut v8::PinScope<'_, '_>,
    database: v8::Local<'_, v8::Object>,
) -> DatabaseHandle {
    indexed_db_native_database_handle(scope, database).expect("materialized SQL database")
}

pub(in crate::context_bootstrap::indexed_db) fn enqueue_native_connection_notifications(
    scope: &mut v8::PinScope<'_, '_>,
) {
    let table = ensure_indexed_db_runtime_state_table(scope);
    let connections = {
        let table = table.borrow();
        table
            .databases
            .values()
            .filter_map(|state| {
                let Some(connection) = &state.backend.connection else {
                    return None;
                };
                state
                    .wrapper
                    .to_local(scope)
                    .map(|database| (database, connection.clone()))
            })
            .collect::<Vec<_>>()
    };
    for (database, connection) in connections {
        while let Some(notification) = connection.take_notification() {
            let task = v8::Object::new(scope);
            register_indexed_db_version_change_task(
                scope,
                task,
                database,
                notification.old_version,
                notification.new_version,
            );
            if let Some(id) = indexed_db_typed_task_id(scope, task) {
                table
                    .borrow_mut()
                    .version_change_tasks
                    .get_mut(&id)
                    .expect("registered versionchange")
                    .completion = Some(notification.completion);
            }
            enqueue_indexed_db_task(scope, task);
        }
    }
}

/// Capture before invoking script: termination cleanup cannot inspect V8.
pub(in crate::context_bootstrap::indexed_db) fn upgrade_event_cancellation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    transaction: v8::Local<'s, v8::Object>,
) -> impl FnOnce() + 'static {
    let connection = indexed_db_database_sql_connection(scope, database);
    let id = indexed_db_typed_state_id(scope, transaction);
    let table = Rc::downgrade(&indexed_db_runtime_state_table_for_object(
        scope,
        transaction,
    ));
    move || {
        if let Some(connection) = connection {
            if let (Some(table), Some(id)) = (table.upgrade(), id)
                && let Some(state) = table.borrow_mut().transactions.get_mut(&id)
                && !state.finished
            {
                // An explicit COMMIT accepted before the interruption keeps
                // its queued native work, just like an ordinary request event.
                if state.committing {
                    return;
                }
                state.active = false;
                state.aborted = true;
                state.cancellation_scheduled = true;
            }
            let _ = connection.retire();
        }
    }
}

/// A forced-close signal is the native rollback receipt. Finish an interrupted
/// upgrade on this later, usable owner task, before its close/open-error events.
pub(in crate::context_bootstrap::indexed_db) fn finish_forced_upgrade<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
) {
    let Some(transaction) = object_property_as_object(
        scope,
        database,
        INDEXED_DB_DATABASE_UPGRADE_TRANSACTION_SLOT,
    ) else {
        return;
    };
    let Some(id) = indexed_db_typed_state_id(scope, transaction) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    let interrupted = table.borrow().transactions.get(&id).is_some_and(|state| {
        state.sql_client.is_some()
            && !state.finished
            && state.aborted
            && state.cancellation_scheduled
    });
    if interrupted {
        sql::disarm_sql_transaction_rollback(scope, transaction);
        let error =
            dom_exception_value(scope, "The upgrade callback was interrupted.", "AbortError");
        finish_transaction_abort_after_rollback(scope, transaction, error);
    } else if !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_FINISHED_SLOT)
        .unwrap_or(false)
        && !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ABORTED_SLOT)
            .unwrap_or(false)
    {
        let error = dom_exception_value(
            scope,
            "The database connection was forcibly closed.",
            "AbortError",
        );
        finish_transaction_abort(scope, transaction, error);
    }
}
