use super::*;
use crate::context_bootstrap::indexed_db::{
    IndexedDbTransactionOperation, IndexedDbValue, ObjectStoreOptions,
    indexed_db_has_pending_tasks, schedule_transaction_operation,
    stores::collection::{CollectionKind, CollectionRequest},
};
use crate::util::get_private_value;
use moli_indexeddb::sqlite::{
    IndexKeys,
    service::{CommandResult, DatabaseConnection, Service, ServiceClient, ServiceConfig},
};
use moli_indexeddb::{IndexOptions, KeyPath};
use std::{cell::Cell, sync::mpsc, time::Duration};

const TX: TransactionHandle = TransactionHandle::from_raw(19);

struct Backend {
    client: DatabaseClient,
    _service: ServiceClient,
    connection: DatabaseConnection,
    buckets: moli_storage_service::SharedStorageBucketStore,
    manager: Arc<parking_lot::Mutex<moli_indexeddb::IndexedDbManager>>,
    storage_scope: IndexedDbStorageScope,
}

impl Drop for Backend {
    fn drop(&mut self) {
        self.manager.lock().shutdown_sqlite().unwrap();
    }
}

impl Backend {
    fn new() -> Self {
        Self::with_store(
            ObjectStoreOptions::default(),
            vec![IndexInfo {
                name: "group".into(),
                key_path: KeyPath::String("group".into()),
                unique: false,
                multi_entry: false,
            }],
        )
    }

    fn with_store(options: ObjectStoreOptions, indexes: Vec<IndexInfo>) -> Self {
        Self::with_store_quota(options, indexes, None)
    }

    fn with_store_quota(
        options: ObjectStoreOptions,
        indexes: Vec<IndexInfo>,
        quota: Option<u64>,
    ) -> Self {
        let config = ServiceConfig::default();

        let service = Service::new(config).unwrap();
        let service_client = service.client();
        let manager = Arc::new(parking_lot::Mutex::new(
            moli_indexeddb::IndexedDbManager::with_sqlite_service(service),
        ));
        let buckets =
            moli_storage_service::new_shared_storage_bucket_store_with_indexed_db_manager(&manager);
        let storage_scope = if let Some(quota) = quota {
            let identity = buckets
                .lock()
                .open_bucket_with_options(
                    storage_scope().storage_key(),
                    "limited",
                    None,
                    None,
                    Some(quota),
                    None,
                )
                .unwrap();
            IndexedDbStorageScope::new(
                identity.indexed_db_storage_key(),
                "test-browser",
                "test-partition",
            )
            .with_bucket_identity(identity)
        } else {
            storage_scope()
        };
        let (sender, receiver) = mpsc::channel();
        let client = service_client
            .resolve(
                storage_scope.storage_key().as_bytes().to_vec(),
                "sql-routing".into(),
                move |reply| {
                    let _ = sender.send(reply);
                },
            )
            .unwrap();
        receive(&receiver).result.unwrap();
        let (sender, receiver) = mpsc::channel();
        client
            .open_connection(Arc::new(|| {}), move |reply| {
                let _ = sender.send(reply);
            })
            .unwrap();
        let CommandResult::Connection(connection) = receive(&receiver).result.unwrap() else {
            panic!("expected connection receipt");
        };
        let backend = Self {
            client,
            _service: service_client,
            connection,
            buckets,
            manager,
            storage_scope,
        };
        let (sender, receiver) = mpsc::channel();
        backend
            .connection
            .begin_upgrade(TX, 1, move |reply| {
                let _ = sender.send(reply);
            })
            .unwrap();
        receive(&receiver).result.unwrap();
        backend.execute(Command::CreateStore {
            transaction: TX,
            name: "records".into(),
            options,
        });
        for index in indexes {
            backend.execute(Command::CreateIndex {
                transaction: TX,
                store: "records".into(),
                name: index.name,
                options: IndexOptions {
                    key_path: index.key_path,
                    unique: index.unique,
                    multi_entry: index.multi_entry,
                },
            });
        }
        backend.execute(Command::Commit {
            transaction: TX,
            quota: None,
        });
        backend.begin(TX);
        backend
    }

    fn execute(&self, command: Command) -> CommandResult {
        let (sender, receiver) = mpsc::channel();
        self.client
            .submit(command, move |reply| {
                let _ = sender.send(reply);
            })
            .unwrap();
        receive(&receiver).result.unwrap()
    }

    fn begin(&self, transaction: TransactionHandle) {
        let stores = vec![moli_indexeddb::IndexedDbName::from("records")];
        let admission = self
            .connection
            .queue_transaction_start(&stores, TransactionMode::ReadWrite, Arc::new(|| {}))
            .unwrap();
        assert!(admission.handle().is_ready());
        self.execute(Command::Begin {
            transaction,
            mode: TransactionMode::ReadWrite,
            stores: stores.into_iter().collect(),
            version: None,
            admission: Some(admission.handle().clone()),
        });
    }

    fn count(&self) -> Command {
        Command::Count {
            transaction: TX,
            store: "records".into(),
            source: Default::default(),
            range: Default::default(),
        }
    }

    fn barrier(&self) {
        let (sender, receiver) = mpsc::channel();
        self.client
            .submit(self.count(), move |_| {
                sender.send(()).unwrap();
            })
            .unwrap();
        receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("SQL worker reached the barrier");
    }

    fn put(&self, value: IndexedDbValue) {
        self.execute(Command::Write {
            transaction: TX,
            store: "records".into(),
            key: Some(Key::from("primary")),
            value,
            indexes: vec![IndexKeys {
                name: "group".into(),
                keys: vec![Key::from("group-key")],
            }],
            add_only: false,
            quota: None,
        });
    }

    fn block(&self) -> WorkerGate {
        let (entered, waiting) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        self.client
            .submit(self.count(), move |_| {
                entered.send(()).unwrap();
                resume
                    .recv_timeout(Duration::from_secs(10))
                    .expect("test releases SQL worker");
            })
            .unwrap();
        waiting.recv_timeout(Duration::from_secs(10)).unwrap();
        WorkerGate(Some(release))
    }
}

struct WorkerGate(Option<mpsc::Sender<()>>);
impl Drop for WorkerGate {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

fn receive(receiver: &mpsc::Receiver<Completion>) -> Completion {
    receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("bounded SQL worker completion")
}

struct SqlWake(RefCell<Option<tokio::sync::mpsc::UnboundedReceiver<()>>>);

fn runtime(scope: &mut v8::PinScope<'_, '_>, backend: &Backend) {
    // These transport tests need an internal queue, without materializing the
    // Window/Worker Web API prototypes in a bare V8 context.
    let state = v8::Object::new(scope);
    let factory = v8::Object::new(scope);
    let global = scope.get_current_context().global(scope);
    set_private_value(scope, global, "moli.IndexedDb.runtimeState", state.into());
    set_private_value(
        scope,
        state,
        "moli.IndexedDb.runtime.factory",
        factory.into(),
    );
    define_non_enumerable_bool_property(
        scope,
        state,
        "moli.IndexedDb.runtime.factoryInitialized",
        true,
    );
    assert!(indexed_db_runtime_array(scope, IndexedDbRuntimeArray::TaskQueue).is_some());
    let context = scope.get_current_context();
    let (wake, receiver) = tokio::sync::mpsc::unbounded_channel();
    set_worker_indexed_db_task_wake_for_context(context, wake);
    context.set_slot(Rc::new(SqlWake(RefCell::new(Some(receiver)))));
    crate::context_bootstrap::storage_buckets::set_storage_bucket_store_for_context(
        context,
        Some(backend.buckets.clone()),
    );
}

fn source<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
    let source = v8::Object::new(scope);
    register_indexed_db_wrapper(scope, source, IndexedDbWrapperKind::Factory, None);
    source
}

fn transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    backend: &Backend,
    started: bool,
) -> (v8::Local<'s, v8::Object>, v8::Local<'s, v8::Object>) {
    let database = v8::Object::new(scope);
    register_indexed_db_wrapper(
        scope,
        database,
        IndexedDbWrapperKind::Database,
        Some(backend.storage_scope.clone()),
    );
    register_indexed_db_database_lifecycle(
        scope,
        database,
        backend.connection.handle(),
        backend.storage_scope.clone(),
        backend.client.clone(),
    );
    attach_indexed_db_sql_connection(scope, database, backend.connection.clone()).unwrap();
    let transaction = v8::Object::new(scope);
    register_indexed_db_wrapper(
        scope,
        transaction,
        IndexedDbWrapperKind::Transaction,
        Some(backend.storage_scope.clone()),
    );
    register_indexed_db_transaction_lifecycle(
        scope,
        transaction,
        database,
        Some(TX),
        TransactionMode::ReadWrite,
        started,
    );
    let id = indexed_db_typed_state_id(scope, transaction).unwrap();
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    table
        .borrow_mut()
        .transactions
        .get_mut(&id)
        .unwrap()
        .sql_client = Some(backend.client.clone());
    (database, transaction)
}

fn request_for_store<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    store: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let storage_scope = indexed_db_typed_storage_scope(scope, transaction);
    let request = v8::Object::new(scope);
    register_indexed_db_wrapper(scope, request, IndexedDbWrapperKind::Request, storage_scope);
    register_indexed_db_request_lifecycle(scope, request, store.into(), transaction.into(), false);
    request
}

fn request<'s>(scope: &mut v8::PinScope<'s, '_>, backend: &Backend) -> v8::Local<'s, v8::Object> {
    let (_, transaction) = transaction(scope, backend, true);
    let store = v8::Object::new(scope);
    register_indexed_db_wrapper(
        scope,
        store,
        IndexedDbWrapperKind::ObjectStore,
        Some(backend.storage_scope.clone()),
    );
    let database = indexed_db_transaction_database(scope, transaction).unwrap();
    register_indexed_db_object_store_lifecycle(
        scope,
        store,
        transaction,
        database,
        IndexedDbObjectStoreMetadata::from_sql(
            &"records".into(),
            &backend.client.committed().unwrap().stores
                [&moli_indexeddb::IndexedDbName::from("records")],
        ),
    );
    request_for_store(scope, transaction, store)
}

fn write_store<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    backend: &Backend,
    started: bool,
) -> (v8::Local<'s, v8::Object>, v8::Local<'s, v8::Object>) {
    let (database, transaction) = transaction(scope, backend, started);
    let store = v8::Object::new(scope);
    register_indexed_db_wrapper(
        scope,
        store,
        IndexedDbWrapperKind::ObjectStore,
        Some(backend.storage_scope.clone()),
    );
    register_indexed_db_object_store_lifecycle(
        scope,
        store,
        transaction,
        database,
        IndexedDbObjectStoreMetadata::from_sql(
            &"records".into(),
            &backend.client.committed().unwrap().stores
                [&moli_indexeddb::IndexedDbName::from("records")],
        ),
    );
    (transaction, store)
}

fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
    let source = v8::String::new(scope, source).unwrap();
    let script = v8::Script::compile(scope, source, None).unwrap();
    crate::script_execution::execute_compiled_script(scope, script).unwrap()
}

fn prepare_write<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
    transaction: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) -> IndexedDbTransactionOperation {
    IndexedDbTransactionOperation::ObjectStoreWrite {
        prepared: crate::context_bootstrap::indexed_db::prepare_object_store_write(
            scope,
            store,
            transaction,
            value,
            ObjectStoreKey::Missing,
        )
        .unwrap(),
        add_only: false,
    }
}

fn drain_sql(scope: &mut v8::PinScope<'_, '_>, backend: &Backend) -> usize {
    let table = ensure_indexed_db_runtime_state_table(scope);
    let wake = scope.get_current_context().get_slot::<SqlWake>().unwrap();
    let mut receiver = wake.0.borrow_mut().take().unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let mut completions = 0;
    loop {
        let id = table.borrow().sql_tasks.keys().next().copied();
        let Some(id) = id else {
            wake.0.borrow_mut().replace(receiver);
            return completions;
        };
        if !backend.client.is_retired() {
            backend.barrier();
        }
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let ready = {
                        let table = table.borrow();
                        table
                            .sql_tasks
                            .get(&id)
                            .unwrap()
                            .completion
                            .lock()
                            .is_some()
                    };
                    if ready {
                        break;
                    }
                    receiver
                        .recv()
                        .await
                        .expect("SQL/quota completion wakes its owner");
                }
            })
            .await
            .expect("bounded SQL/quota completion");
        });
        assert!(flush_indexed_db_task_by_id(scope, id));
        completions += 1;
        assert!(
            completions < 100,
            "bounded fixture drains accepted SQL operations"
        );
    }
}

fn storage_scope() -> IndexedDbStorageScope {
    let storage_key = moli_storage_key::MoliStorageKey::first_party_from_url(
        &url::Url::parse("https://sql.example").unwrap(),
        None,
    )
    .serialized_storage_key();
    IndexedDbStorageScope::new(storage_key, "test-browser", "test-partition")
}

#[test]
fn metadata_refresh_rejects_a_database_without_a_sql_client() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let database = v8::Object::new(scope);
    assert!(matches!(
        crate::context_bootstrap::indexed_db::refresh_database_surface(scope, database),
        Err(IndexedDbError::InvalidState(_))
    ));
}

#[test]
fn quota_requires_a_logical_connection_in_the_transport_fixture() {
    crate::ensure_v8_for_test();
    let backend = Backend::new();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    runtime(scope, &backend);
    let (database, transaction) = transaction(scope, &backend, true);
    let id = indexed_db_typed_state_id(scope, database).unwrap();
    let table = ensure_indexed_db_runtime_state_table(scope);
    let connection = table
        .borrow_mut()
        .databases
        .get_mut(&id)
        .unwrap()
        .backend
        .connection
        .take()
        .unwrap();
    assert!(matches!(
        crate::context_bootstrap::indexed_db::storage_bucket::sql_quota_owner(scope, transaction),
        Err(IndexedDbError::InvalidState(_))
    ));
    attach_indexed_db_sql_connection(scope, database, connection).unwrap();
    assert!(
        crate::context_bootstrap::indexed_db::storage_bucket::sql_quota_owner(scope, transaction)
            .unwrap()
            .is_some()
    );
}

#[test]
fn transport_writes_enforce_real_bucket_quota_and_restore_generated_keys() {
    crate::ensure_v8_for_test();
    let backend = Backend::with_store_quota(
        ObjectStoreOptions {
            auto_increment: true,
            ..Default::default()
        },
        Vec::new(),
        Some(1024),
    );
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    runtime(scope, &backend);
    let (transaction, store) = write_store(scope, &backend, true);
    let mut requests = Vec::new();
    for bytes in [4096, 8] {
        let request = request_for_store(scope, transaction, store);
        schedule(
            scope,
            request,
            IndexedDbTransactionOperation::ObjectStoreWrite {
                prepared: crate::context_bootstrap::indexed_db::PreparedObjectStoreWrite {
                    key: crate::context_bootstrap::indexed_db::PreparedObjectStoreKey::Generate,
                    value: vec![1; bytes].into(),
                },
                add_only: false,
            },
        );
        requests.push(request);
    }
    assert_eq!(drain_sql(scope, &backend), 2);
    let error = object_hidden_value(scope, requests[0], INDEXED_DB_PENDING_ERROR_SLOT)
        .unwrap()
        .to_object(scope)
        .unwrap();
    // Bare transport contexts do not install DOMException's JS prototype.
    // The clone helper checks the native QuotaExceededError brand and fields.
    let (_, quota, requested) =
        crate::context_bootstrap::quota_exceeded_error_clone_fields(scope, error)
            .expect("the request contains a branded quota error");
    assert_eq!(quota, Some(1024.0));
    assert!(requested.is_some_and(|requested| requested > 1024.0));
    assert_eq!(result(scope, requests[1]).number_value(scope), Some(1.0));
    assert!(matches!(
        backend.execute(backend.count()),
        CommandResult::Count(1)
    ));
}

#[test]
fn rejected_deferred_commit_waits_for_a_native_rollback_receipt() {
    crate::ensure_v8_for_test();
    let backend = Backend::new();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    runtime(scope, &backend);
    let (_, transaction) = transaction(scope, &backend, true);
    let table = ensure_indexed_db_runtime_state_table(scope);
    let transaction_id = indexed_db_typed_state_id(scope, transaction).unwrap();
    table
        .borrow_mut()
        .transactions
        .get_mut(&transaction_id)
        .unwrap()
        .sql_rollback = Some(SqlTransactionRollback::new(backend.client.clone(), TX));
    let gate = backend.block();

    let client = backend.client.clone();
    submit_sql_reply_task(
        scope,
        transaction,
        move |complete| {
            complete(Err(IndexedDbError::Resource(
                "quota dispatch rejected".into(),
            )));
            Ok(client)
        },
        |scope, source, reply| abort_sql_dispatch(scope, source, reply.err().unwrap()),
    )
    .unwrap();
    let ticket = *table.borrow().sql_tasks.keys().next().unwrap();
    assert!(flush_indexed_db_task_by_id(scope, ticket));
    {
        let state = table.borrow();
        let transaction = &state.transactions[&transaction_id];
        assert!(transaction.aborted);
        assert!(
            !transaction.finished,
            "a dispatch rejection is not a native rollback receipt"
        );
        assert!(
            !state
                .tasks
                .values()
                .any(|task| task.kind == IndexedDbTaskKind::TransactionAbort)
        );
    }
    drop(gate);
    let (finished, observed) = mpsc::channel();
    backend
        .client
        .submit(backend.count(), move |_| {
            let _ = finished.send(());
        })
        .unwrap();
    observed.recv_timeout(Duration::from_secs(10)).unwrap();
    let rollback = *table.borrow().sql_tasks.keys().next().unwrap();
    assert!(flush_indexed_db_task_by_id(scope, rollback));
    assert!(table.borrow().transactions[&transaction_id].finished);
    assert!(
        table
            .borrow()
            .tasks
            .values()
            .any(|task| task.kind == IndexedDbTaskKind::TransactionAbort)
    );

    backend.begin(TX);
    assert!(matches!(
        backend.execute(backend.count()),
        CommandResult::Count(0)
    ));
    backend.execute(Command::Abort { transaction: TX });
}

#[test]
fn rejected_sql_request_after_native_close_settles_request_before_transaction_abort() {
    crate::ensure_v8_for_test();
    let backend = Backend::new();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    runtime(scope, &backend);
    let request = request(scope, &backend);
    let transaction = source_transaction(scope, request).unwrap();
    ensure_sql_transaction_rollback(scope, request).unwrap();
    backend.execute(Command::Close);
    assert!(backend.client.is_retired());
    abort_sql_dispatch(
        scope,
        request,
        IndexedDbError::Resource("backend closed during quota wait".into()),
    );
    let table = ensure_indexed_db_runtime_state_table(scope);
    let id = indexed_db_typed_state_id(scope, transaction).unwrap();
    assert!(table.borrow().transactions[&id].finished);
    assert!(table.borrow().transactions[&id].sql_rollback.is_none());
    assert!(object_hidden_value(scope, request, INDEXED_DB_PENDING_ERROR_SLOT).is_some());
    let queue = indexed_db_runtime_array(scope, IndexedDbRuntimeArray::TaskQueue).unwrap();
    let kinds = (0..queue.length())
        .filter_map(|index| {
            let task = queue.get_index(scope, index)?.to_object(scope)?;
            indexed_db_typed_task_kind(scope, task)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        [
            IndexedDbTaskKind::RequestError,
            IndexedDbTaskKind::TransactionAbort
        ]
    );
    assert!(get_private_value(scope, transaction, SQL_ABORT_REQUEST).is_none());
}

#[test]
fn inline_write_closed_after_prepare_settles_abort_after_request_error() {
    crate::ensure_v8_for_test();
    let options = ObjectStoreOptions {
        key_path: Some(KeyPath::String("id".into())),
        auto_increment: true,
    };
    let backend = Backend::with_store(options.clone(), Vec::new());
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    runtime(scope, &backend);
    let (transaction, store) = write_store(scope, &backend, true);
    let request = request_for_store(scope, transaction, store);
    let value = eval(scope, "({saved:true})");
    let prepared = prepare_write(scope, store, transaction, value);
    schedule(scope, request, prepared);
    // The SQL prepare has allocated an inline key, but V8 has not consumed its
    // completion. Native cleanup wins before finish/cancel can be submitted.
    backend.barrier();
    let table = ensure_indexed_db_runtime_state_table(scope);
    let task = *table.borrow().sql_tasks.keys().next().unwrap();
    backend.execute(Command::Close);
    assert!(backend.client.is_retired());
    assert!(flush_indexed_db_task_by_id(scope, task));
    drain_sql(scope, &backend);
    let id = indexed_db_typed_state_id(scope, transaction).unwrap();
    assert!(table.borrow().transactions[&id].finished);
    assert!(table.borrow().transactions[&id].aborted);
    assert!(table.borrow().transactions[&id].sql_rollback.is_none());
    let queue = indexed_db_runtime_array(scope, IndexedDbRuntimeArray::TaskQueue).unwrap();
    let kinds = (0..queue.length())
        .filter_map(|index| {
            let task = queue.get_index(scope, index)?.to_object(scope)?;
            indexed_db_typed_task_kind(scope, task)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        [
            IndexedDbTaskKind::RequestError,
            IndexedDbTaskKind::TransactionAbort
        ]
    );
    assert!(get_private_value(scope, transaction, SQL_ABORT_REQUEST).is_none());
}

fn schedule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    kind: IndexedDbTransactionOperation,
) {
    let transaction = indexed_db_request_transaction_object(scope, request).unwrap();
    let source = object_hidden_value(scope, request, INDEXED_DB_REQUEST_SOURCE_SLOT)
        .unwrap()
        .to_object(scope)
        .unwrap();
    schedule_transaction_operation(scope, transaction, source, request, &"records".into(), kind);
}

fn result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Value> {
    if let Some(value) = object_hidden_value(scope, request, INDEXED_DB_PENDING_RESULT_SLOT) {
        return value;
    }
    let error = object_hidden_value(scope, request, INDEXED_DB_PENDING_ERROR_SLOT);
    let description = error.map(|error| error.to_rust_string_lossy(scope));
    let ready = object_string_property(scope, request, INDEXED_DB_REQUEST_READY_STATE_SLOT);
    let final_value = object_hidden_value(scope, request, INDEXED_DB_REQUEST_RESULT_SLOT)
        .map(|value| value.to_rust_string_lossy(scope));
    panic!(
        "missing SQL request result; pending error: {description:?}, ready: {ready:?}, final result: {final_value:?}"
    );
}

fn finish_sql(scope: &mut v8::PinScope<'_, '_>, backend: &Backend) {
    let table = ensure_indexed_db_runtime_state_table(scope);
    let id = *table.borrow().sql_tasks.keys().next().unwrap();
    backend.barrier();
    assert!(flush_indexed_db_task_by_id(scope, id));
    assert!(table.borrow().sql_tasks.is_empty());
}

#[test]
fn sql_read_results_materialize_without_a_legacy_manager_or_record_scan() {
    crate::ensure_v8_for_test();
    let backend = Backend::new();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    runtime(scope, &backend);
    let poison = v8str(
        scope,
        "Object.defineProperty(Array.prototype, '0', {set() {throw new Error('internal queue invoked author setter')}, configurable: true})",
    );
    let script = v8::Script::compile(scope, poison, None).unwrap();
    crate::script_execution::execute_compiled_script(scope, script).unwrap();
    let value = v8str(scope, "from SQLite");
    backend.put(serialize_js_value(scope, value.into()).unwrap());
    let query = IdbKeyRangeQuery::default();

    let get = request(scope, &backend);
    schedule(
        scope,
        get,
        IndexedDbTransactionOperation::ObjectStoreGet {
            query: query.clone(),
        },
    );
    finish_sql(scope, &backend);
    assert_eq!(
        result(scope, get).to_rust_string_lossy(scope),
        "from SQLite"
    );
    // Materialization is separate from the subsequent request success event.
    assert_eq!(
        object_string_property(scope, get, INDEXED_DB_REQUEST_READY_STATE_SLOT).as_deref(),
        Some("pending")
    );

    let get_key = request(scope, &backend);
    schedule(
        scope,
        get_key,
        IndexedDbTransactionOperation::ObjectStoreGetKey {
            query: query.clone(),
        },
    );
    finish_sql(scope, &backend);
    assert_eq!(
        result(scope, get_key).to_rust_string_lossy(scope),
        "primary"
    );

    let count = request(scope, &backend);
    schedule(
        scope,
        count,
        IndexedDbTransactionOperation::ObjectStoreCount { query: None },
    );
    finish_sql(scope, &backend);
    assert_eq!(result(scope, count).number_value(scope), Some(1.0));

    for kind in [CollectionKind::Value, CollectionKind::Key] {
        let get_all = request(scope, &backend);
        schedule(
            scope,
            get_all,
            IndexedDbTransactionOperation::ObjectStoreGetAll(CollectionRequest {
                query: None,
                count: None,
                direction: CursorDirection::Prev,
                kind,
            }),
        );
        finish_sql(scope, &backend);
        let array = v8::Local::<v8::Array>::try_from(result(scope, get_all)).unwrap();
        assert_eq!(array.length(), 1);
        let expected = if matches!(kind, CollectionKind::Key) {
            "primary"
        } else {
            "from SQLite"
        };
        assert_eq!(
            array
                .get_index(scope, 0)
                .unwrap()
                .to_rust_string_lossy(scope),
            expected
        );
    }
    let index_key = request(scope, &backend);
    let store = object_hidden_value(scope, index_key, INDEXED_DB_REQUEST_SOURCE_SLOT)
        .unwrap()
        .to_object(scope)
        .unwrap();
    let index = v8::Object::new(scope);
    register_indexed_db_wrapper(
        scope,
        index,
        IndexedDbWrapperKind::Index,
        Some(backend.storage_scope.clone()),
    );
    register_indexed_db_index_lifecycle(
        scope,
        index,
        store,
        &IndexInfo {
            name: "group".into(),
            key_path: KeyPath::String("group".into()),
            unique: false,
            multi_entry: false,
        },
    )
    .unwrap();
    set_private_value(
        scope,
        index_key,
        INDEXED_DB_REQUEST_SOURCE_SLOT,
        index.into(),
    );
    schedule(
        scope,
        index_key,
        IndexedDbTransactionOperation::IndexGetKey {
            query: IdbKeyRangeQuery {
                lower: Some(Key::from("group-key")),
                upper: Some(Key::from("group-key")),
                ..Default::default()
            },
        },
    );
    finish_sql(scope, &backend);
    assert_eq!(
        result(scope, index_key).to_rust_string_lossy(scope),
        "primary"
    );
}

#[test]
fn waiting_sql_tasks_do_not_spin_or_block_ready_owner_tasks() {
    crate::ensure_v8_for_test();
    let backend = Backend::new();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    runtime(scope, &backend);
    let (wake, mut wakes) = tokio::sync::mpsc::unbounded_channel();
    set_worker_indexed_db_task_wake_for_context(context, wake);
    let gate = backend.block();
    let source = source(scope);
    let completed = Rc::new(Cell::new(false));
    let observed = completed.clone();
    submit_indexed_db_sql_task(
        scope,
        source,
        &backend.client,
        backend.count(),
        move |_, _, reply| {
            assert!(matches!(reply.result, Ok(CommandResult::Count(0))));
            observed.set(true);
        },
    )
    .unwrap();
    let table = ensure_indexed_db_runtime_state_table(scope);
    let id = *table.borrow().sql_tasks.keys().next().unwrap();
    assert!(!indexed_db_has_pending_tasks(scope));
    assert!(!flush_next_indexed_db_task(scope));
    assert!(!flush_indexed_db_task_by_id(scope, id));
    assert!(wakes.try_recv().is_err());
    // An unrelated ready task may run while the SQL command is queued.
    let task = v8::Object::new(scope);
    register_indexed_db_task(scope, task, IndexedDbTaskKind::DatabasesSettle, None);
    push_object_to_indexed_db_runtime_array(scope, IndexedDbRuntimeArray::TaskQueue, task);
    assert!(
        pop_first_indexed_db_task(scope)
            .unwrap()
            .strict_equals(task.into())
    );
    unregister_indexed_db_task(scope, task);
    drop(gate);
    backend.barrier();
    assert!(wakes.try_recv().is_ok());
    assert!(indexed_db_has_pending_tasks(scope));
    assert!(flush_next_indexed_db_task(scope));
    assert!(completed.get());
    assert!(!flush_next_indexed_db_task(scope));
}

#[test]
fn sql_completion_returns_to_the_accepting_realm() {
    crate::ensure_v8_for_test();
    let backend = Backend::new();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let first = v8::Context::new(scope, Default::default());
    let second = v8::Context::new(scope, Default::default());
    let source = {
        let scope = &mut v8::ContextScope::new(scope, first);
        runtime(scope, &backend);
        source(scope)
    };
    let completed = Rc::new(Cell::new(false));
    {
        let scope = &mut v8::ContextScope::new(scope, second);
        runtime(scope, &backend);
        let observed = completed.clone();
        submit_indexed_db_sql_task(
            scope,
            source,
            &backend.client,
            backend.count(),
            move |scope, _, _| {
                let global = scope.get_current_context().global(scope);
                assert!(
                    get_private_value(scope, global, "sqlRealmMarker")
                        .unwrap()
                        .is_true()
                );
                observed.set(true);
            },
        )
        .unwrap();
        assert_eq!(scope.get_current_context(), second);
        backend.barrier();
        assert!(!flush_next_indexed_db_task(scope));
    }
    {
        let scope = &mut v8::ContextScope::new(scope, first);
        let global = first.global(scope);
        let marker = v8::Boolean::new(scope, true);
        set_private_value(scope, global, "sqlRealmMarker", marker.into());
        assert!(flush_next_indexed_db_task(scope));
    }
    assert!(completed.get());
}

#[test]
fn discarding_sql_tasks_drops_ready_and_late_completions() {
    crate::ensure_v8_for_test();
    for ready in [false, true] {
        let backend = Backend::new();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        runtime(scope, &backend);
        let gate = (!ready).then(|| backend.block());
        let source = source(scope);
        submit_indexed_db_sql_task(
            scope,
            source,
            &backend.client,
            backend.count(),
            |_, _, _| panic!("discarded SQL tasks must not deliver results"),
        )
        .unwrap();
        if ready {
            backend.barrier();
        }
        let table = ensure_indexed_db_runtime_state_table(scope);
        let receiver =
            Arc::downgrade(&table.borrow().sql_tasks.values().next().unwrap().completion);
        let id = *table.borrow().sql_tasks.keys().next().unwrap();
        assert!(discard_indexed_db_task_by_id(scope, id));
        assert!(receiver.upgrade().is_none());
        drop(gate);
        backend.barrier();
        assert!(table.borrow().sql_tasks.is_empty());
    }
}

#[test]
fn malformed_sql_wire_is_a_request_error_instead_of_successful_undefined() {
    crate::ensure_v8_for_test();
    let backend = Backend::new();
    backend.put(vec![1, 2, 3].into());
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    runtime(scope, &backend);
    let get_key = request(scope, &backend);
    schedule(
        scope,
        get_key,
        IndexedDbTransactionOperation::ObjectStoreGetKey {
            query: IdbKeyRangeQuery::default(),
        },
    );
    finish_sql(scope, &backend);
    assert_eq!(
        result(scope, get_key).to_rust_string_lossy(scope),
        "primary"
    );
    let request = request(scope, &backend);
    schedule(
        scope,
        request,
        IndexedDbTransactionOperation::ObjectStoreGet {
            query: IdbKeyRangeQuery::default(),
        },
    );
    finish_sql(scope, &backend);
    assert!(object_hidden_value(scope, request, INDEXED_DB_PENDING_ERROR_SLOT).is_some());
    assert!(object_hidden_value(scope, request, INDEXED_DB_PENDING_RESULT_SLOT).is_none());
}

#[test]
fn sql_delete_commands_use_the_same_dispatcher_and_return_undefined() {
    crate::ensure_v8_for_test();
    let backend = Backend::new();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    runtime(scope, &backend);
    for kind in [
        IndexedDbTransactionOperation::CursorDelete {
            key: Key::from("primary"),
        },
        IndexedDbTransactionOperation::ObjectStoreDelete {
            query: IdbKeyRangeQuery {
                lower: Some(Key::from("primary")),
                upper: Some(Key::from("primary")),
                ..Default::default()
            },
        },
        IndexedDbTransactionOperation::ObjectStoreClear,
    ] {
        backend.put(vec![1, 2, 3].into());
        let request = request(scope, &backend);
        schedule(scope, request, kind);
        finish_sql(scope, &backend);
        assert!(result(scope, request).is_undefined());
        assert!(matches!(
            backend.execute(backend.count()),
            CommandResult::Count(0)
        ));
    }
}

#[test]
fn inline_sql_writes_keep_their_round_trips_in_fifo_and_index_the_snapshot() {
    crate::ensure_v8_for_test();
    let options = ObjectStoreOptions {
        key_path: Some(KeyPath::String("id".into())),
        auto_increment: true,
    };
    let indexes = vec![
        IndexInfo {
            name: "group".into(),
            key_path: KeyPath::String("group".into()),
            unique: false,
            multi_entry: false,
        },
        IndexInfo {
            name: "id".into(),
            key_path: KeyPath::String("id".into()),
            unique: false,
            multi_entry: false,
        },
        IndexInfo {
            name: "tags".into(),
            key_path: KeyPath::String("tags".into()),
            unique: false,
            multi_entry: true,
        },
    ];
    let backend = Backend::with_store(options.clone(), indexes.clone());
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    runtime(scope, &backend);
    let (transaction, store) = write_store(scope, &backend, true);
    let gate = backend.block();
    let first = request_for_store(scope, transaction, store);
    let value = eval(
        scope,
        "globalThis.input = {group: 'first', tags: ['x', 'x', 'y']}",
    );
    let prepared = prepare_write(scope, store, transaction, value);
    schedule(scope, first, prepared);
    eval(scope, "input.group = 'mutated'; input.tags.push('late')");
    let second = request_for_store(scope, transaction, store);
    let value = eval(scope, "({group: 'second', tags: []})");
    let prepared = prepare_write(scope, store, transaction, value);
    schedule(scope, second, prepared);
    let count = request_for_store(scope, transaction, store);
    schedule(
        scope,
        count,
        IndexedDbTransactionOperation::ObjectStoreCount { query: None },
    );
    let table = ensure_indexed_db_runtime_state_table(scope);
    assert_eq!(table.borrow().sql_tasks.len(), 1);
    let id = indexed_db_typed_state_id(scope, transaction).unwrap();
    assert_eq!(table.borrow().transactions[&id].operations.len(), 2);
    assert!(object_hidden_value(scope, first, INDEXED_DB_PENDING_RESULT_SLOT).is_none());
    drop(gate);
    assert_eq!(
        drain_sql(scope, &backend),
        5,
        "each inline write uses prepare/finish before the read"
    );
    assert_eq!(result(scope, first).number_value(scope), Some(1.0));
    assert_eq!(result(scope, second).number_value(scope), Some(2.0));
    assert_eq!(result(scope, count).number_value(scope), Some(2.0));
    assert!(!table.borrow().transactions[&id].operation_in_flight);
    for (name, expected) in [
        ("group", vec![Key::from("first"), Key::from("second")]),
        ("id", vec![Key::from(1_i64), Key::from(2_i64)]),
        ("tags", vec![Key::from("x"), Key::from("y")]),
    ] {
        let CommandResult::Records(records) = backend.execute(Command::Query {
            transaction: TX,
            store: "records".into(),
            query: moli_indexeddb::sqlite::Query {
                source: moli_indexeddb::sqlite::QuerySource::Index(name.into()),
                keys_only: true,
                limit: u32::MAX,
                ..Default::default()
            },
        }) else {
            panic!("SQL index results")
        };
        assert_eq!(
            records
                .into_iter()
                .map(|record| record.key)
                .collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
fn sql_unique_failure_and_owner_decode_failure_roll_back_generated_keys_before_next_write() {
    crate::ensure_v8_for_test();
    let options = ObjectStoreOptions {
        key_path: Some(KeyPath::String("id".into())),
        auto_increment: true,
    };
    let indexes = vec![IndexInfo {
        name: "group".into(),
        key_path: KeyPath::String("group".into()),
        unique: true,
        multi_entry: false,
    }];
    let backend = Backend::with_store(options.clone(), indexes.clone());
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    runtime(scope, &backend);
    let (transaction, store) = write_store(scope, &backend, true);
    let mut requests = Vec::new();
    for value in [
        "({group: 'duplicate'})",
        "({group: 'duplicate'})",
        "({group: 'different'})",
    ] {
        let request = request_for_store(scope, transaction, store);
        let value = eval(scope, value);
        let prepared = prepare_write(scope, store, transaction, value);
        schedule(scope, request, prepared);
        requests.push(request);
        if requests.len() == 2 {
            let invalid = request_for_store(scope, transaction, store);
            schedule(scope, invalid, IndexedDbTransactionOperation::ObjectStoreWrite {
                prepared: crate::context_bootstrap::indexed_db::PreparedObjectStoreWrite {
                    key: crate::context_bootstrap::indexed_db::PreparedObjectStoreKey::GenerateInline { key_path: "id".into() },
                    value: vec![1, 2, 3].into(),
                }, add_only: false,
            });
            requests.push(invalid);
        }
    }
    assert_eq!(drain_sql(scope, &backend), 8);
    assert_eq!(result(scope, requests[0]).number_value(scope), Some(1.0));
    for request in &requests[1..3] {
        assert!(object_hidden_value(scope, *request, INDEXED_DB_PENDING_ERROR_SLOT).is_some());
        assert!(object_hidden_value(scope, *request, INDEXED_DB_PENDING_RESULT_SLOT).is_none());
    }
    assert_eq!(result(scope, requests[3]).number_value(scope), Some(2.0));
    assert!(matches!(
        backend.execute(backend.count()),
        CommandResult::Count(2)
    ));
}

#[test]
fn index_free_sql_writes_use_one_command_and_reuse_the_prepared_bytes() {
    crate::ensure_v8_for_test();
    let options = ObjectStoreOptions {
        key_path: None,
        auto_increment: true,
    };
    let backend = Backend::with_store(options.clone(), Vec::new());
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    runtime(scope, &backend);
    let (transaction, store) = write_store(scope, &backend, true);
    let mut requests = Vec::new();
    for key in [
        crate::context_bootstrap::indexed_db::PreparedObjectStoreKey::Generate,
        crate::context_bootstrap::indexed_db::PreparedObjectStoreKey::Explicit(Key::from(100_i64)),
        crate::context_bootstrap::indexed_db::PreparedObjectStoreKey::Generate,
    ] {
        let request = request_for_store(scope, transaction, store);
        // Opaque bytes make accidental decoding fail rather than silently pass.
        schedule(
            scope,
            request,
            IndexedDbTransactionOperation::ObjectStoreWrite {
                prepared: crate::context_bootstrap::indexed_db::PreparedObjectStoreWrite {
                    key,
                    value: vec![1, 2, 3].into(),
                },
                add_only: false,
            },
        );
        requests.push(request);
    }
    assert_eq!(drain_sql(scope, &backend), 3);
    for (request, expected) in requests.into_iter().zip([1.0, 100.0, 101.0]) {
        assert_eq!(result(scope, request).number_value(scope), Some(expected));
    }
}

#[test]
fn sql_operations_wait_for_start_and_keep_their_snapshot() {
    crate::ensure_v8_for_test();
    let options = ObjectStoreOptions {
        key_path: None,
        auto_increment: true,
    };
    let backend = Backend::with_store(options.clone(), Vec::new());
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    runtime(scope, &backend);
    let (transaction, store) = write_store(scope, &backend, false);
    let write = request_for_store(scope, transaction, store);
    let value = eval(scope, "'snapshot'.repeat(8192)");
    let prepared = prepare_write(scope, store, transaction, value);
    schedule(scope, write, prepared);
    let count = request_for_store(scope, transaction, store);
    schedule(
        scope,
        count,
        IndexedDbTransactionOperation::ObjectStoreCount { query: None },
    );
    let table = ensure_indexed_db_runtime_state_table(scope);
    assert!(table.borrow().sql_tasks.is_empty());
    let id = indexed_db_typed_state_id(scope, transaction).unwrap();
    assert_eq!(table.borrow().transactions[&id].operations.len(), 2);
    assert!(matches!(
        backend.execute(backend.count()),
        CommandResult::Count(0)
    ));
    table
        .borrow_mut()
        .transactions
        .get_mut(&id)
        .unwrap()
        .started = true;
    crate::context_bootstrap::indexed_db::pump_transaction_operations(scope, transaction);
    assert_eq!(drain_sql(scope, &backend), 2);
    assert_eq!(result(scope, write).number_value(scope), Some(1.0));
    assert_eq!(result(scope, count).number_value(scope), Some(1.0));
}

#[test]
fn request_preparation_errors_wait_for_start_and_preceding_inline_writes() {
    crate::ensure_v8_for_test();
    for started in [false, true] {
        let backend = Backend::with_store(
            ObjectStoreOptions {
                key_path: Some(KeyPath::String("id".into())),
                auto_increment: true,
            },
            Vec::new(),
        );
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        runtime(scope, &backend);
        let (transaction, store) = write_store(scope, &backend, started);
        let first = request_for_store(scope, transaction, store);
        let value = eval(scope, "({saved: true})");
        let prepared = prepare_write(scope, store, transaction, value);
        schedule(scope, first, prepared);

        // A store wrapper has no index metadata: preparation fails before any
        // SQL submission. The already-created request must keep its FIFO place.
        let second = request_for_store(scope, transaction, store);
        schedule(
            scope,
            second,
            IndexedDbTransactionOperation::IndexGet {
                query: Default::default(),
            },
        );
        assert!(object_hidden_value(scope, first, INDEXED_DB_PENDING_RESULT_SLOT).is_none());
        assert!(object_hidden_value(scope, second, INDEXED_DB_PENDING_ERROR_SLOT).is_none());

        if !started {
            let table = ensure_indexed_db_runtime_state_table(scope);
            assert!(table.borrow().sql_tasks.is_empty());
            let id = indexed_db_typed_state_id(scope, transaction).unwrap();
            table
                .borrow_mut()
                .transactions
                .get_mut(&id)
                .unwrap()
                .started = true;
            crate::context_bootstrap::indexed_db::pump_transaction_operations(scope, transaction);
        }
        assert_eq!(drain_sql(scope, &backend), 2);
        assert_eq!(result(scope, first).number_value(scope), Some(1.0));
        assert!(object_hidden_value(scope, second, INDEXED_DB_PENDING_ERROR_SLOT).is_some());
        let queue = indexed_db_runtime_array(scope, IndexedDbRuntimeArray::TaskQueue).unwrap();
        let kinds = (0..queue.length())
            .filter_map(|index| {
                let task = queue.get_index(scope, index)?.to_object(scope)?;
                indexed_db_typed_task_kind(scope, task)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            [
                IndexedDbTaskKind::RequestSuccess,
                IndexedDbTaskKind::RequestError
            ]
        );
    }
}

#[test]
fn sql_inline_cancellation_rolls_back_before_the_next_write() {
    crate::ensure_v8_for_test();
    let options = ObjectStoreOptions {
        key_path: Some(KeyPath::String("id".into())),
        auto_increment: true,
    };
    let backend = Backend::with_store(options.clone(), Vec::new());
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    runtime(scope, &backend);
    let (transaction, store) = write_store(scope, &backend, true);
    let invalid = request_for_store(scope, transaction, store);
    schedule(
        scope,
        invalid,
        IndexedDbTransactionOperation::ObjectStoreWrite {
            prepared: crate::context_bootstrap::indexed_db::PreparedObjectStoreWrite {
                key: crate::context_bootstrap::indexed_db::PreparedObjectStoreKey::GenerateInline {
                    key_path: "id".into(),
                },
                value: vec![1, 2, 3].into(),
            },
            add_only: false,
        },
    );
    let valid = request_for_store(scope, transaction, store);
    let value = eval(scope, "({saved: true})");
    let prepared = prepare_write(scope, store, transaction, value);
    schedule(scope, valid, prepared);
    backend.barrier();

    assert_eq!(drain_sql(scope, &backend), 4);
    assert!(object_hidden_value(scope, invalid, INDEXED_DB_PENDING_ERROR_SLOT).is_some());
    assert_eq!(result(scope, valid).number_value(scope), Some(1.0));

    assert!(matches!(
        backend.execute(backend.count()),
        CommandResult::Count(1)
    ));
}

#[test]
fn discarded_inline_prepare_aborts_native_transaction() {
    crate::ensure_v8_for_test();
    let options = ObjectStoreOptions {
        key_path: Some(KeyPath::String("id".into())),
        auto_increment: true,
    };
    let backend = Backend::with_store(options.clone(), Vec::new());
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    runtime(scope, &backend);
    let (transaction, store) = write_store(scope, &backend, true);
    for _ in 0..2 {
        let request = request_for_store(scope, transaction, store);
        let value = eval(scope, "({saved: true})");
        let prepared = prepare_write(scope, store, transaction, value);
        schedule(scope, request, prepared);
    }
    backend.barrier();
    let table = ensure_indexed_db_runtime_state_table(scope);
    let task = *table.borrow().sql_tasks.keys().next().unwrap();

    assert!(discard_indexed_db_task_by_id(scope, task));
    backend.barrier();
    let id = indexed_db_typed_state_id(scope, transaction).unwrap();
    assert!(table.borrow().transactions[&id].aborted);
    assert!(table.borrow().transactions[&id].sql_rollback.is_none());
    assert!(table.borrow().transactions[&id].sql_client.is_none());
    assert!(table.borrow().sql_tasks.is_empty());

    let next = TransactionHandle::from_raw(20);
    backend.begin(next);
    assert!(matches!(
        backend.execute(Command::Count {
            transaction: next,
            store: "records".into(),
            source: Default::default(),
            range: Default::default()
        }),
        CommandResult::Count(0)
    ));
    let value = eval(scope, "({id: 1})");
    let value = serialize_js_value(scope, value).unwrap();
    let CommandResult::Key(key) = backend.execute(Command::Write {
        transaction: next,
        store: "records".into(),
        key: None,
        value,
        indexes: Vec::new(),
        add_only: false,
        quota: None,
    }) else {
        panic!("generated key after rollback")
    };
    assert_eq!(key, Key::from(1_i64));
}

#[test]
fn retiring_sql_context_releases_transaction_clients_and_rolls_back() {
    crate::ensure_v8_for_test();
    let backend = Backend::new();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    runtime(scope, &backend);
    let (_, transaction) = transaction(scope, &backend, true);
    ensure_sql_transaction_rollback(scope, transaction).unwrap();
    let table = ensure_indexed_db_runtime_state_table(scope);
    let id = indexed_db_typed_state_id(scope, transaction).unwrap();
    assert!(table.borrow().transactions[&id].sql_client.is_some());
    assert!(table.borrow().transactions[&id].sql_rollback.is_some());
    retire_indexed_db_context(context);
    assert!(table.borrow().transactions[&id].sql_client.is_none());
    assert!(table.borrow().transactions[&id].sql_rollback.is_none());
    // The rollback was enqueued before connection retirement, even though the
    // wrapper and its realm are both still alive.
    let (sender, receiver) = mpsc::channel();
    backend
        .client
        .submit(backend.count(), move |reply| {
            let _ = sender.send(reply);
        })
        .unwrap();
    assert!(matches!(
        receive(&receiver).result,
        Err(IndexedDbError::TransactionInactive(_))
    ));
}

#[test]
fn discarded_schema_preparation_error_releases_its_sql_owner() {
    crate::ensure_v8_for_test();
    let backend = Backend::new();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    runtime(scope, &backend);
    let (_, transaction) = transaction(scope, &backend, true);
    ensure_sql_transaction_rollback(scope, transaction).unwrap();
    // Inject a failed schema preparation into the common FIFO.

    crate::context_bootstrap::indexed_db::schedule_schema_failure(
        scope,
        transaction,
        IndexedDbError::Resource("unable to prepare schema metadata".into()),
    );
    let table = ensure_indexed_db_runtime_state_table(scope);
    let task = table
        .borrow()
        .object_tasks
        .iter()
        .find_map(|(id, payload)| payload.failure.as_ref().map(|_| *id))
        .unwrap();
    assert!(discard_indexed_db_task_by_id(scope, task));
    backend.barrier();

    let next = TransactionHandle::from_raw(20);
    backend.begin(next);
    assert!(matches!(
        backend.execute(Command::Count {
            transaction: next,
            store: "records".into(),
            source: Default::default(),
            range: Default::default(),
        }),
        CommandResult::Count(0)
    ));
}

#[test]
fn schema_preparation_error_waits_for_the_whole_preceding_inline_write() {
    crate::ensure_v8_for_test();
    let options = ObjectStoreOptions {
        key_path: Some(KeyPath::String("id".into())),
        auto_increment: true,
    };
    let backend = Backend::with_store(options.clone(), Vec::new());
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    runtime(scope, &backend);
    let (transaction, store) = write_store(scope, &backend, true);
    let request = request_for_store(scope, transaction, store);
    let value = eval(scope, "({saved: true})");
    let prepared = prepare_write(scope, store, transaction, value);
    schedule(scope, request, prepared);
    backend.barrier();
    // Inject a failed schema preparation into the common FIFO.

    crate::context_bootstrap::indexed_db::schedule_schema_failure(
        scope,
        transaction,
        IndexedDbError::Resource("unable to prepare schema metadata".into()),
    );

    assert_eq!(drain_sql(scope, &backend), 2);
    assert_eq!(result(scope, request).number_value(scope), Some(1.0));
    assert!(matches!(
        backend.execute(backend.count()),
        CommandResult::Count(1)
    ));
    let table = ensure_indexed_db_runtime_state_table(scope);
    let task = table
        .borrow()
        .object_tasks
        .iter()
        .find_map(|(id, payload)| payload.failure.as_ref().map(|_| *id))
        .unwrap();
    assert!(flush_indexed_db_task_by_id(scope, task));
    assert_eq!(drain_sql(scope, &backend), 1);
    let next = TransactionHandle::from_raw(20);
    backend.begin(next);
    assert!(matches!(
        backend.execute(Command::Count {
            transaction: next,
            store: "records".into(),
            source: Default::default(),
            range: Default::default(),
        }),
        CommandResult::Count(0)
    ));
}

fn interrupt_index_extraction(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    scope.terminate_execution();
}

#[test]
fn terminated_index_extraction_aborts_sql_and_does_not_start_the_next_operation() {
    crate::ensure_v8_for_test();
    let options = ObjectStoreOptions {
        key_path: Some(KeyPath::String("id".into())),
        auto_increment: true,
    };
    let indexes = vec![IndexInfo {
        name: "tags".into(),
        key_path: KeyPath::String("tags".into()),
        unique: false,
        multi_entry: true,
    }];
    let backend = Backend::with_store(options.clone(), indexes.clone());
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    runtime(scope, &backend);
    let (transaction, store) = write_store(scope, &backend, true);
    for value in ["({tags: new Array(1)})", "({tags: []})"] {
        let request = request_for_store(scope, transaction, store);
        let value = eval(scope, value);
        let prepared = prepare_write(scope, store, transaction, value);
        schedule(scope, request, prepared);
    }
    backend.barrier();
    let interrupt = v8::Function::new(scope, interrupt_index_extraction).unwrap();
    let name = v8str(scope, "interrupt");
    context
        .global(scope)
        .set(scope, name.into(), interrupt.into())
        .unwrap();
    eval(
        scope,
        "Object.defineProperty(Array.prototype, '0', {get() { interrupt(); while (true) {} }, configurable: true})",
    );
    let table = ensure_indexed_db_runtime_state_table(scope);
    let task = *table.borrow().sql_tasks.keys().next().unwrap();
    {
        v8::tc_scope!(let scope, scope);
        assert!(flush_indexed_db_task_by_id(scope, task));
        assert!(scope.has_terminated());
        assert!(!scope.can_continue());
    }
    scope.cancel_terminate_execution();
    eval(scope, "delete Array.prototype[0]");
    backend.barrier();
    let id = indexed_db_typed_state_id(scope, transaction).unwrap();
    assert!(table.borrow().transactions[&id].aborted);
    assert!(table.borrow().transactions[&id].operations.is_empty());
    assert!(!table.borrow().transactions[&id].operation_in_flight);
    assert!(table.borrow().sql_tasks.is_empty());

    let next = TransactionHandle::from_raw(20);
    backend.begin(next);
    assert!(matches!(
        backend.execute(Command::Count {
            transaction: next,
            store: "records".into(),
            source: Default::default(),
            range: Default::default()
        }),
        CommandResult::Count(0)
    ));
}
#[test]
fn sql_collection_batches_keep_one_operation_in_flight_until_the_whole_result_is_ready() {
    fn flush_one(scope: &mut v8::PinScope<'_, '_>, backend: &Backend) {
        let table = ensure_indexed_db_runtime_state_table(scope);
        let id = *table.borrow().sql_tasks.keys().next().unwrap();
        backend.barrier();
        assert!(flush_indexed_db_task_by_id(scope, id));
    }

    crate::ensure_v8_for_test();
    let backend = Backend::with_store(ObjectStoreOptions::default(), Vec::new());
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    runtime(scope, &backend);
    let large = eval(scope, "'x'.repeat(2 * 1024 * 1024)");
    let large = serialize_js_value(scope, large).unwrap();
    let small = eval(scope, "'small'");
    let small = serialize_js_value(scope, small).unwrap();
    for (key, value) in [(1, large), (2, small)] {
        backend.execute(Command::Write {
            transaction: TX,
            store: "records".into(),
            key: Some(Key::from(key as i64)),
            value,
            indexes: Vec::new(),
            add_only: false,
            quota: None,
        });
    }
    let (transaction, store) = write_store(scope, &backend, true);
    let all = request_for_store(scope, transaction, store);
    schedule(
        scope,
        all,
        IndexedDbTransactionOperation::ObjectStoreGetAll(CollectionRequest {
            query: None,
            count: None,
            direction: CursorDirection::Next,
            kind: CollectionKind::Value,
        }),
    );
    let deletion = request_for_store(scope, transaction, store);
    schedule(
        scope,
        deletion,
        IndexedDbTransactionOperation::ObjectStoreDelete {
            query: crate::context_bootstrap::indexed_db::IdbKeyRangeQuery {
                lower: Some(2.into()),
                upper: Some(2.into()),
                ..Default::default()
            },
        },
    );
    flush_one(scope, &backend);
    assert!(object_hidden_value(scope, all, INDEXED_DB_PENDING_RESULT_SLOT).is_none());
    assert!(object_hidden_value(scope, deletion, INDEXED_DB_PENDING_RESULT_SLOT).is_none());
    assert!(matches!(
        backend.execute(backend.count()),
        CommandResult::Count(2)
    ));
    flush_one(scope, &backend);
    let array = v8::Local::<v8::Array>::try_from(result(scope, all)).unwrap();
    assert_eq!(array.length(), 2);
    let second = array.get_index(scope, 1).unwrap();
    assert_eq!(second.to_rust_string_lossy(scope), "small");
    assert!(object_hidden_value(scope, deletion, INDEXED_DB_PENDING_RESULT_SLOT).is_none());
    assert_eq!(drain_sql(scope, &backend), 1);
    assert!(matches!(
        backend.execute(backend.count()),
        CommandResult::Count(1)
    ));
    let queue = indexed_db_runtime_array(scope, IndexedDbRuntimeArray::TaskQueue).unwrap();
    let kinds = (0..queue.length())
        .filter_map(|index| {
            let task = queue.get_index(scope, index)?.to_object(scope)?;
            indexed_db_typed_task_kind(scope, task)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        [
            IndexedDbTaskKind::RequestSuccess,
            IndexedDbTaskKind::RequestSuccess
        ]
    );
}
