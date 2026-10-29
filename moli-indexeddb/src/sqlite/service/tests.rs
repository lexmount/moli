use super::*;
use crate::{DatabaseHandle, TransactionRequestLease, TransactionRequestStatus};
use moli_sqlite::Capacity;
use std::{sync::mpsc, time::Duration};

mod catalog_tests;
mod clear_tests;
mod connection_tests;
mod metadata_tests;
mod quota_tests;
mod registration_tests;

const TX: TransactionHandle = TransactionHandle::from_raw(11);

fn config() -> ServiceConfig {
    ServiceConfig {
        executor: ExecutorConfig {
            workers: 1,
            data: Capacity {
                tasks: 16,
                bytes: 512 * 1024,
            },
            control: Capacity {
                tasks: 8,
                bytes: 64 * 1024,
            },
        },
        connection: ConnectionOptions {
            cache_kib: 128,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn identity(name: &str) -> DatabaseIdentity {
    DatabaseIdentity {
        owner_key: b"partition/default/storage-key/top-site/bucket-17".to_vec(),
        name: name.into(),
        generation: [42; 16],
    }
}

fn receive(receiver: &mpsc::Receiver<Completion>) -> Completion {
    receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("SQL worker must complete without waiting for the JS owner")
}

fn register(service: &Service, name: &str, location: Location) -> (DatabaseClient, Completion) {
    let (sender, receiver) = mpsc::channel();
    let client = service
        .open(identity(name), location, move |reply| {
            let _ = sender.send(reply);
        })
        .unwrap();
    (client, receive(&receiver))
}

fn open(service: &Service, name: &str, location: Location) -> DatabaseClient {
    let (client, reply) = register(service, name, location);
    assert!(matches!(reply.result.unwrap(), CommandResult::Metadata(_)));
    client
}

#[test]
fn transaction_handles_are_shared_across_clients_preserve_u64_and_do_not_wrap() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "handles", Location::Memory);
    let clone = client.clone();
    client
        .state
        .next_transaction
        .store(1u64 << 53, Ordering::Relaxed);
    assert_eq!(
        client.allocate_transaction_handle().unwrap().into_raw(),
        (1u64 << 53) + 1
    );
    assert_eq!(
        clone.allocate_transaction_handle().unwrap().into_raw(),
        (1u64 << 53) + 2
    );
    client
        .state
        .next_transaction
        .store(u64::MAX - 1, Ordering::Relaxed);
    assert_eq!(
        clone.allocate_transaction_handle().unwrap().into_raw(),
        u64::MAX
    );
    assert!(matches!(
        client.allocate_transaction_handle(),
        Err(IndexedDbError::Resource(_))
    ));
    assert!(matches!(
        clone.allocate_transaction_handle(),
        Err(IndexedDbError::Resource(_))
    ));
}

fn reply(client: &DatabaseClient, command: Command) -> Completion {
    let (sender, receiver) = mpsc::channel();
    client
        .submit(command, 4096, move |reply| {
            let _ = sender.send(reply);
        })
        .unwrap();
    receive(&receiver)
}

fn execute(client: &DatabaseClient, command: Command) -> Result<CommandResult, IndexedDbError> {
    reply(client, command).result
}

fn initialize(client: &DatabaseClient) {
    execute(
        client,
        Command::Begin {
            transaction: TX,
            mode: TransactionMode::VersionChange,
            stores: BTreeSet::new(),
            version: Some(1),
            admission: None,
        },
    )
    .unwrap();
    for name in ["a", "b"] {
        execute(
            client,
            Command::CreateStore {
                transaction: TX,
                name: name.into(),
                options: ObjectStoreOptions {
                    auto_increment: true,
                    ..Default::default()
                },
            },
        )
        .unwrap();
    }
    execute(
        client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
    assert_eq!(client.committed().unwrap().version, 1);
}

fn queue(client: &DatabaseClient, store: &str, mode: TransactionMode) -> TransactionRequestLease {
    client
        .queue_transaction_start(
            DatabaseHandle::from_raw(7),
            &[store.into()],
            mode,
            Arc::new(|| {}),
        )
        .unwrap()
}

fn start(
    client: &DatabaseClient,
    store: &str,
    mode: TransactionMode,
    lease: &TransactionRequestLease,
) {
    execute(
        client,
        Command::Begin {
            transaction: TX,
            mode,
            stores: BTreeSet::from([store.into()]),
            version: None,
            admission: Some(lease.handle().clone()),
        },
    )
    .unwrap();
}

#[test]
fn rejected_commit_rolls_back_an_open_request_before_releasing_admission() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "failed-commit", Location::Memory);
    initialize(&client);
    let first = queue(&client, "a", TransactionMode::ReadWrite);
    let next = queue(&client, "a", TransactionMode::ReadWrite);
    start(&client, "a", TransactionMode::ReadWrite, &first);
    assert!(matches!(
        execute(
            &client,
            Command::PrepareWrite {
                transaction: TX,
                store: "a".into(),
                add_only: false
            }
        )
        .unwrap(),
        CommandResult::Prepared(_)
    ));
    assert!(
        execute(
            &client,
            Command::Commit {
                transaction: TX,
                quota: None
            }
        )
        .is_err()
    );
    assert_eq!(first.handle().status(), TransactionRequestStatus::Finished);
    assert!(next.handle().is_ready());
    start(&client, "a", TransactionMode::ReadWrite, &next);
    let CommandResult::Prepared(prepared) = execute(
        &client,
        Command::PrepareWrite {
            transaction: TX,
            store: "a".into(),
            add_only: false,
        },
    )
    .unwrap() else {
        panic!("prepared generated key");
    };
    assert_eq!(prepared.key, 1.into());
    execute(&client, Command::Abort { transaction: TX }).unwrap();
}

fn write(client: &DatabaseClient, bytes: &[u8]) {
    assert!(matches!(
        execute(
            client,
            Command::Write {
                transaction: TX,
                store: "a".into(),
                key: None,
                value: bytes.to_vec().into(),
                indexes: vec![],
                add_only: false,
                quota: None
            }
        )
        .unwrap(),
        CommandResult::Key(_)
    ));
}

fn query(client: &DatabaseClient) -> Vec<RecordSnapshot> {
    let CommandResult::Records(records) = execute(
        client,
        Command::Query {
            transaction: TX,
            store: "a".into(),
            query: Query::default(),
        },
    )
    .unwrap() else {
        panic!("expected SQL records")
    };
    records
}

fn large_value() -> crate::IndexedDbValue {
    crate::IndexedDbValue::new(
        vec![9; 4097],
        vec![crate::IndexedDbExternalObject::Blob {
            bytes: vec![7; 64 * 1024],
            mime_type: "application/octet-stream".into(),
        }],
    )
}

#[test]
fn query_grows_its_permit_and_retains_the_result_budget_until_delivery() {
    let config = config();
    let service = Service::new(config).unwrap();
    let client = open(&service, "growing-result", Location::Memory);
    initialize(&client);
    let writer = queue(&client, "a", TransactionMode::ReadWrite);
    start(&client, "a", TransactionMode::ReadWrite, &writer);
    execute(
        &client,
        Command::Write {
            transaction: TX,
            store: "a".into(),
            key: None,
            value: large_value(),
            indexes: vec![],
            add_only: false,
            quota: None,
        },
    )
    .unwrap();
    execute(
        &client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
    let reader = queue(&client, "a", TransactionMode::ReadOnly);
    start(&client, "a", TransactionMode::ReadOnly, &reader);
    let command = Command::Query {
        transaction: TX,
        store: "a".into(),
        query: Query::default(),
    };
    // A retained owner snapshot is not available for use as result storage.
    let retained_snapshot_bytes = 8192;
    let initial = command.input_bytes() + retained_snapshot_bytes;
    let permit = client.try_reserve(TaskKind::Data, initial).unwrap();
    let (sender, receiver) = mpsc::channel();
    client
        .submit_reserved(permit, command, 0, move |reply| {
            let _ = sender.send(reply);
        })
        .unwrap();
    let reply = receive(&receiver);
    let CommandResult::Records(records) = reply.result.as_ref().unwrap() else {
        panic!("expected query result")
    };
    assert_eq!(records[0].value, Some(large_value()));
    let reserved = reply.permit.reserved_bytes();
    assert!(reserved >= initial + 4097 + 64 * 1024);
    let rest = client
        .try_reserve(TaskKind::Data, config.executor.data.bytes - reserved)
        .unwrap();
    assert!(matches!(
        client.try_reserve(TaskKind::Data, 1),
        Err(DispatchError::Executor(ExecutorError::Capacity))
    ));
    // Control work can finish while results still occupy the data budget.
    execute(
        &client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
    drop(reply);
    let recovered = client.try_reserve(TaskKind::Data, reserved).unwrap();
    drop((rest, recovered));
}

#[test]
fn query_budget_pressure_does_not_abort_the_transaction_or_load_key_only_bodies() {
    let config = config();
    let service = Service::new(config).unwrap();
    let client = open(&service, "result-pressure", Location::Memory);
    initialize(&client);
    let writer = queue(&client, "a", TransactionMode::ReadWrite);
    start(&client, "a", TransactionMode::ReadWrite, &writer);
    execute(
        &client,
        Command::Write {
            transaction: TX,
            store: "a".into(),
            key: None,
            value: large_value(),
            indexes: vec![],
            add_only: false,
            quota: None,
        },
    )
    .unwrap();
    execute(
        &client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
    let reader = queue(&client, "a", TransactionMode::ReadOnly);
    start(&client, "a", TransactionMode::ReadOnly, &reader);
    let occupied = client
        .try_reserve(TaskKind::Data, config.executor.data.bytes - 16 * 1024)
        .unwrap();
    let (sender, receiver) = mpsc::channel();
    client
        .submit(
            Command::Query {
                transaction: TX,
                store: "a".into(),
                query: Query::default(),
            },
            0,
            move |reply| {
                let _ = sender.send(reply);
            },
        )
        .unwrap();
    let reply = receive(&receiver);
    assert!(matches!(reply.result, Err(IndexedDbError::Resource(_))));
    // The 64 KiB body was not reserved/allocated beyond the remaining 16 KiB.
    assert!(reply.permit.reserved_bytes() <= 16 * 1024);
    let CommandResult::Records(keys) = execute(
        &client,
        Command::Query {
            transaction: TX,
            store: "a".into(),
            query: Query {
                keys_only: true,
                ..Default::default()
            },
        },
    )
    .unwrap() else {
        panic!("expected key-only query result")
    };
    assert_eq!(keys.len(), 1);
    assert!(keys[0].value.is_none());
    drop((reply, occupied));
    assert_eq!(query(&client)[0].value, Some(large_value()));
    execute(
        &client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
}

#[test]
fn two_databases_progress_on_one_worker_while_the_first_waits_for_js() {
    let service = Service::new(config()).unwrap();
    let first = open(&service, "first", Location::Memory);
    let second = open(&service, "second", Location::Memory);
    initialize(&first);
    initialize(&second);
    let first_lease = queue(&first, "a", TransactionMode::ReadWrite);
    start(&first, "a", TransactionMode::ReadWrite, &first_lease);
    write(&first, b"uncommitted");
    let second_lease = queue(&second, "a", TransactionMode::ReadWrite);
    start(&second, "a", TransactionMode::ReadWrite, &second_lease);
    write(&second, b"committed");
    execute(
        &second,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
    assert_eq!(
        first_lease.handle().status(),
        TransactionRequestStatus::Started
    );
    execute(&first, Command::Abort { transaction: TX }).unwrap();
    let reader = queue(&second, "a", TransactionMode::ReadOnly);
    start(&second, "a", TransactionMode::ReadOnly, &reader);
    assert_eq!(query(&second)[0].value, Some(b"committed".to_vec().into()));
    execute(
        &second,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
    let reader = queue(&first, "a", TransactionMode::ReadOnly);
    start(&first, "a", TransactionMode::ReadOnly, &reader);
    assert!(query(&first).is_empty());
    execute(
        &first,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
}

#[test]
fn readonly_and_disjoint_scopes_are_serial_and_keep_fifo_order() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "serial", Location::Memory);
    initialize(&client);
    let first = queue(&client, "a", TransactionMode::ReadOnly);
    let second = queue(&client, "a", TransactionMode::ReadOnly);
    let third = queue(&client, "b", TransactionMode::ReadWrite);
    assert!(first.handle().is_ready());
    assert_eq!(second.handle().status(), TransactionRequestStatus::Pending);
    assert_eq!(third.handle().status(), TransactionRequestStatus::Pending);
    start(&client, "a", TransactionMode::ReadOnly, &first);
    execute(
        &client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
    assert!(second.handle().is_ready());
    assert_eq!(third.handle().status(), TransactionRequestStatus::Pending);
    start(&client, "a", TransactionMode::ReadOnly, &second);
    execute(
        &client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
    assert!(third.handle().is_ready());
    start(&client, "b", TransactionMode::ReadWrite, &third);
    execute(&client, Command::Abort { transaction: TX }).unwrap();
}

#[test]
fn commit_publishes_usage_before_waking_successor_even_without_a_result_receiver() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "publication", Location::Memory);
    initialize(&client);
    let initial = client.committed().unwrap().usage;
    let first = queue(&client, "a", TransactionMode::ReadWrite);
    start(&client, "a", TransactionMode::ReadWrite, &first);
    let first_handle = first.handle().clone();
    drop(first);
    let (sender, receiver) = mpsc::channel();
    let observer = client.clone();
    let second = client
        .queue_transaction_start(
            DatabaseHandle::from_raw(8),
            &["a".into()],
            TransactionMode::ReadOnly,
            Arc::new(move || {
                let _ = sender.send(observer.committed().unwrap().usage);
            }),
        )
        .unwrap();
    write(&client, b"published even if the initiating realm is gone");
    assert_eq!(client.committed().unwrap().usage, initial);
    assert_eq!(first_handle.status(), TransactionRequestStatus::Started);
    client
        .submit(
            Command::Commit {
                transaction: TX,
                quota: None,
            },
            0,
            drop,
        )
        .unwrap();
    let published = receiver.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(published > initial);
    assert_eq!(client.committed().unwrap().usage, published);
    assert!(second.handle().is_ready());
    start(&client, "a", TransactionMode::ReadOnly, &second);
    assert_eq!(
        query(&client)[0].value,
        Some(
            b"published even if the initiating realm is gone"
                .to_vec()
                .into()
        )
    );
    execute(
        &client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
}

#[test]
fn private_memory_databases_survive_more_than_eight_connections_on_one_worker() {
    let service = Service::new(config()).unwrap();
    let mut clients = Vec::new();
    for index in 0..16 {
        let name = format!("memory-{index}");
        let client = open(&service, &name, Location::Memory);
        initialize(&client);
        let lease = queue(&client, "a", TransactionMode::ReadWrite);
        start(&client, "a", TransactionMode::ReadWrite, &lease);
        write(&client, name.as_bytes());
        execute(
            &client,
            Command::Commit {
                transaction: TX,
                quota: None,
            },
        )
        .unwrap();
        clients.push(client);
    }
    for client in clients {
        let reopened = open(&service, &client.identity().name, Location::Memory);
        assert_eq!(reopened.id(), client.id());
        let reader = queue(&reopened, "a", TransactionMode::ReadOnly);
        start(&reopened, "a", TransactionMode::ReadOnly, &reader);
        let records = query(&reopened);
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].value,
            Some(client.identity().name.as_bytes().to_vec().into())
        );
        execute(
            &reopened,
            Command::Commit {
                transaction: TX,
                quota: None,
            },
        )
        .unwrap();
    }
}

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let mut token = [0; 16];
        moli_crypto::fill_secure_random(&mut token).unwrap();
        let directory = Self(std::env::temp_dir().join(format!(
            "moli-idb-service-{}",
            moli_crypto::sha256_hex(token)
        )));
        std::fs::create_dir(&directory.0).unwrap();
        directory
    }
    fn location(&self, name: &str) -> Location {
        Location::Disk {
            path: self.0.join(name),
            mode: OpenMode::Create,
        }
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn active_disk_transactions_allow_more_than_eight_databases_on_one_worker() {
    let directory = Directory::new();
    let mut service = Service::new(config()).unwrap();
    let mut clients = Vec::new();
    for index in 0..16 {
        let name = format!("disk-{index}");
        let client = open(
            &service,
            &name,
            directory.location(&format!("{name}.sqlite")),
        );
        initialize(&client);
        let lease = queue(&client, "a", TransactionMode::ReadWrite);
        start(&client, "a", TransactionMode::ReadWrite, &lease);
        write(&client, name.as_bytes());
        clients.push((client, lease));
    }
    // Every database above still owns an active transaction on the same worker.
    for (index, (client, _lease)) in clients.iter().enumerate() {
        let (reopened, reply) =
            register(&service, &client.identity().name, client.location.clone());
        assert_eq!(reopened.id(), client.id());
        reply.result.unwrap();
        assert_eq!(reply.active_transaction, Some(TX));
        assert_eq!(query(client).len(), 1);
        let command = if index % 2 == 0 {
            Command::Commit {
                transaction: TX,
                quota: None,
            }
        } else {
            Command::Abort { transaction: TX }
        };
        execute(client, command).unwrap();
    }
    service.shutdown().unwrap();
    let service = Service::new(config()).unwrap();
    for (index, (client, _lease)) in clients.iter().enumerate() {
        let Location::Disk { path, .. } = &client.location else {
            panic!("disk database");
        };
        let reopened = open(
            &service,
            &client.identity().name,
            Location::Disk {
                path: path.clone(),
                mode: OpenMode::Existing,
            },
        );
        let reader = queue(&reopened, "a", TransactionMode::ReadOnly);
        start(&reopened, "a", TransactionMode::ReadOnly, &reader);
        let records = query(&reopened);
        assert_eq!(records.len(), usize::from(index % 2 == 0));
        if let Some(record) = records.first() {
            assert_eq!(
                record.value,
                Some(client.identity().name.as_bytes().to_vec().into())
            );
        }
        execute(
            &reopened,
            Command::Commit {
                transaction: TX,
                quota: None,
            },
        )
        .unwrap();
    }
}

#[test]
fn closed_disk_connection_does_not_recreate_a_missing_database() {
    let directory = Directory::new();
    let service = Service::new(config()).unwrap();
    let first = open(&service, "first", directory.location("first.sqlite"));
    initialize(&first);
    execute(&first, Command::Close).unwrap();
    let path = directory.0.join("first.sqlite");
    std::fs::remove_file(&path).unwrap();
    let (_, reply) = register(
        &service,
        "first",
        Location::Disk {
            path: path.clone(),
            mode: OpenMode::Existing,
        },
    );
    assert!(reply.result.is_err());
    assert!(
        !path.exists(),
        "a previously identified database must never be recreated"
    );
}

#[test]
fn control_commands_and_shutdown_do_not_wait_for_data_capacity() {
    let settings = config();
    let mut service = Service::new(settings).unwrap();
    let client = open(&service, "control", Location::Memory);
    initialize(&client);
    let first = queue(&client, "a", TransactionMode::ReadWrite);
    start(&client, "a", TransactionMode::ReadWrite, &first);
    write(&client, b"rollback");
    let second = queue(&client, "a", TransactionMode::ReadOnly);
    let data = client
        .try_reserve(TaskKind::Data, settings.executor.data.bytes)
        .unwrap();
    execute(&client, Command::Abort { transaction: TX }).unwrap();
    assert!(second.handle().is_ready());
    let control = client
        .try_reserve(TaskKind::Control, settings.executor.control.bytes)
        .unwrap();
    service.shutdown().unwrap();
    assert!(client.is_closed());
    assert_eq!(second.handle().status(), TransactionRequestStatus::Canceled);
    assert_eq!(service.queue_usage().data_tasks, 1);
    assert_eq!(service.queue_usage().control_tasks, 1);
    drop((data, control));
    assert_eq!(service.queue_usage(), moli_sqlite::QueueUsage::default());
}

#[test]
fn inline_key_round_trip_reuses_the_snapshot_permit_until_result_consumption() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "inline", Location::Memory);
    initialize(&client);
    let initial = client.committed().unwrap().usage;
    let lease = queue(&client, "a", TransactionMode::ReadWrite);
    start(&client, "a", TransactionMode::ReadWrite, &lease);
    let permit = client.try_reserve(TaskKind::Data, 16 * 1024).unwrap();
    let (sender, receiver) = mpsc::channel();
    client
        .submit_reserved(
            permit,
            Command::PrepareWrite {
                transaction: TX,
                store: "a".into(),
                add_only: false,
            },
            4096,
            move |reply| {
                let _ = sender.send(reply);
            },
        )
        .unwrap();
    let prepared = receive(&receiver);
    let CommandResult::Prepared(write) = prepared.result.unwrap() else {
        panic!("expected prepared write")
    };
    assert_eq!(write.key, 1.into());
    assert_eq!(service.queue_usage().data_tasks, 1);
    let (sender, receiver) = mpsc::channel();
    client
        .submit_reserved(
            prepared.permit,
            Command::FinishWrite {
                transaction: TX,
                request: write.id,
                value: b"final inline snapshot".to_vec().into(),
                indexes: vec![],
                quota: None,
            },
            4096,
            move |reply| {
                let _ = sender.send(reply);
            },
        )
        .unwrap();
    let result = receive(&receiver);
    assert!(matches!(result.result, Ok(CommandResult::Key(_))));
    assert_eq!(service.queue_usage().data_tasks, 1);
    assert_eq!(client.committed().unwrap().usage, initial);
    execute(
        &client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
    assert!(client.committed().unwrap().usage > initial);
    drop(result);
    assert_eq!(service.queue_usage().data_tasks, 0);
}

#[test]
fn reservations_from_other_services_wrong_classes_or_small_bounds_are_rejected() {
    let service = Service::new(config()).unwrap();
    let other = Service::new(config()).unwrap();
    let client = open(&service, "first", Location::Memory);
    let other_client = open(&other, "second", Location::Memory);
    let foreign = other_client.try_reserve(TaskKind::Control, 4096).unwrap();
    assert!(matches!(
        client.submit_reserved(foreign, Command::Abort { transaction: TX }, 0, drop),
        Err(DispatchError::InvalidReservation)
    ));
    let wrong = client.try_reserve(TaskKind::Data, 4096).unwrap();
    assert!(matches!(
        client.submit_reserved(wrong, Command::Abort { transaction: TX }, 0, drop),
        Err(DispatchError::InvalidReservation)
    ));
    let small = client.try_reserve(TaskKind::Control, 1).unwrap();
    assert!(matches!(
        client.submit_reserved(small, Command::Abort { transaction: TX }, 0, drop),
        Err(DispatchError::InvalidReservation)
    ));
    assert_eq!(service.queue_usage(), moli_sqlite::QueueUsage::default());
    assert_eq!(other.queue_usage(), moli_sqlite::QueueUsage::default());
}

#[test]
fn pending_admission_is_bounded_and_withdrawal_releases_capacity() {
    let mut settings = config();
    settings.admissions = 2;
    let service = Service::new(settings).unwrap();
    let client = open(&service, "bounded", Location::Memory);
    initialize(&client);
    let first = queue(&client, "a", TransactionMode::ReadOnly);
    let second = queue(&client, "b", TransactionMode::ReadOnly);
    assert!(matches!(
        client.queue_transaction_start(
            DatabaseHandle::from_raw(8),
            &["a".into()],
            TransactionMode::ReadOnly,
            Arc::new(|| {})
        ),
        Err(IndexedDbError::Resource(_))
    ));
    drop(first);
    assert!(second.handle().is_ready());
    let third = queue(&client, "a", TransactionMode::ReadOnly);
    assert_eq!(third.handle().status(), TransactionRequestStatus::Pending);
    drop((second, third));
    let next = queue(&client, "a", TransactionMode::ReadOnly);
    assert!(next.handle().is_ready());
}

#[test]
fn closing_and_registering_admission_cannot_leave_a_live_lease() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "close-race", Location::Memory);
    initialize(&client);
    let guard = client.state.admission_gate.lock();
    let barrier = std::sync::Barrier::new(2);
    let (sender, receiver) = mpsc::channel();
    let admission = std::thread::scope(|scope| {
        let thread = scope.spawn(|| {
            barrier.wait();
            client.queue_transaction_start(
                DatabaseHandle::from_raw(7),
                &["a".into()],
                TransactionMode::ReadOnly,
                Arc::new(|| {}),
            )
        });
        barrier.wait();
        let sent = client.submit(Command::Close, 0, move |reply| {
            let _ = sender.send(reply);
        });
        drop(guard);
        sent.unwrap();
        receive(&receiver).result.unwrap();
        thread.join().unwrap()
    });
    if let Ok(lease) = admission {
        assert_eq!(lease.handle().status(), TransactionRequestStatus::Canceled);
    }
    assert!(client.is_closed());
    assert!(
        client
            .queue_transaction_start(
                DatabaseHandle::from_raw(7),
                &["a".into()],
                TransactionMode::ReadOnly,
                Arc::new(|| {})
            )
            .is_err()
    );
}

#[test]
fn stale_or_foreign_admission_cannot_start_another_database() {
    let service = Service::new(config()).unwrap();
    let first = open(&service, "first", Location::Memory);
    let second = open(&service, "second", Location::Memory);
    initialize(&first);
    initialize(&second);
    let first_lease = queue(&first, "a", TransactionMode::ReadWrite);
    let wrong = || Command::Begin {
        transaction: TX,
        mode: TransactionMode::ReadWrite,
        stores: BTreeSet::from(["a".into()]),
        version: None,
        admission: Some(first_lease.handle().clone()),
    };
    assert!(matches!(
        execute(&second, wrong()),
        Err(IndexedDbError::InvalidState(_))
    ));
    assert!(first_lease.handle().is_ready());
    start(&first, "a", TransactionMode::ReadWrite, &first_lease);
    write(&first, b"rollback on close");
    let pending = queue(&first, "a", TransactionMode::ReadOnly);
    execute(&first, Command::Close).unwrap();
    assert_eq!(
        first_lease.handle().status(),
        TransactionRequestStatus::Canceled
    );
    assert_eq!(
        pending.handle().status(),
        TransactionRequestStatus::Canceled
    );
    assert!(matches!(
        first.submit(Command::Abort { transaction: TX }, 0, drop),
        Err(DispatchError::Closed)
    ));
    let reopened = open(&service, "first", Location::Memory);
    assert_ne!(first.id(), reopened.id());
    initialize(&reopened);
    assert!(matches!(
        execute(&reopened, wrong()),
        Err(IndexedDbError::InvalidState(_))
    ));
    let reader = queue(&reopened, "a", TransactionMode::ReadOnly);
    start(&reopened, "a", TransactionMode::ReadOnly, &reader);
    assert!(query(&reopened).is_empty());
    execute(
        &reopened,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
}

#[test]
fn duplicate_identity_reuses_client_but_generation_and_path_conflicts_are_rejected() {
    let directory = Directory::new();
    let service = Service::new(config()).unwrap();
    let client = open(&service, "same", directory.location("same.sqlite"));
    initialize(&client);
    let same = open(&service, "same", directory.location("same.sqlite"));
    assert_eq!(same.id(), client.id());
    let mut changed = identity("same");
    changed.generation[0] ^= 1;
    assert!(matches!(
        service.open(changed, directory.location("same.sqlite"), drop),
        Err(DispatchError::IdentityConflict)
    ));
    assert!(matches!(
        service.open(identity("another"), directory.location("same.sqlite"), drop),
        Err(DispatchError::IdentityConflict)
    ));
    assert!(matches!(
        service.open(identity("same"), directory.location("another.sqlite"), drop),
        Err(DispatchError::IdentityConflict)
    ));
    let reader = queue(&client, "a", TransactionMode::ReadOnly);
    start(&client, "a", TransactionMode::ReadOnly, &reader);
    assert!(query(&client).is_empty());
    execute(
        &client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
}

#[test]
fn admission_byte_capacity_rejects_without_using_command_slots() {
    let mut settings = config();
    settings.admission_bytes = 1;
    let service = Service::new(settings).unwrap();
    let client = open(&service, "admission-bytes", Location::Memory);
    initialize(&client);
    assert!(matches!(
        client.queue_transaction_start(
            DatabaseHandle::from_raw(7),
            &["a".into()],
            TransactionMode::ReadOnly,
            Arc::new(|| {})
        ),
        Err(IndexedDbError::Resource(_))
    ));
    assert_eq!(service.queue_usage(), moli_sqlite::QueueUsage::default());
}

#[test]
fn recoverable_write_failure_and_inline_cancellation_keep_transaction_admission() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "request-rollback", Location::Memory);
    initialize(&client);
    let initial = client.committed().unwrap().usage;
    let first = queue(&client, "a", TransactionMode::ReadWrite);
    start(&client, "a", TransactionMode::ReadWrite, &first);
    let successor = queue(&client, "a", TransactionMode::ReadOnly);
    let failed = execute(
        &client,
        Command::Write {
            transaction: TX,
            store: "a".into(),
            key: None,
            value: b"fails quota".to_vec().into(),
            indexes: vec![],
            add_only: false,
            quota: Some(0),
        },
    );
    assert!(matches!(failed, Err(IndexedDbError::QuotaExceeded { .. })));
    assert!(query(&client).is_empty());
    assert_eq!(first.handle().status(), TransactionRequestStatus::Started);
    assert_eq!(
        successor.handle().status(),
        TransactionRequestStatus::Pending
    );
    let CommandResult::Prepared(prepared) = execute(
        &client,
        Command::PrepareWrite {
            transaction: TX,
            store: "a".into(),
            add_only: false,
        },
    )
    .unwrap() else {
        panic!("expected inline key")
    };
    assert_eq!(prepared.key, 1.into());
    execute(
        &client,
        Command::CancelWrite {
            transaction: TX,
            request: prepared.id,
        },
    )
    .unwrap();
    assert_eq!(
        successor.handle().status(),
        TransactionRequestStatus::Pending
    );
    write(&client, b"survives");
    assert_eq!(client.committed().unwrap().usage, initial);
    execute(
        &client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
    assert!(successor.handle().is_ready());
    start(&client, "a", TransactionMode::ReadOnly, &successor);
    let records = query(&client);
    assert_eq!(records[0].primary_key, 1.into());
    assert_eq!(records[0].value, Some(b"survives".to_vec().into()));
    execute(
        &client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
}

fn with_database(client: &DatabaseClient, operation: impl FnOnce(&mut Database) + Send + 'static) {
    let lane = client.lane.clone();
    let id = client.id;
    let (sender, receiver) = mpsc::channel();
    client
        .try_reserve(TaskKind::Control, 0)
        .unwrap()
        .submit(id.0, move |permit| {
            operation(
                lane.lock()
                    .entries
                    .get_mut(&id)
                    .unwrap()
                    .database
                    .as_mut()
                    .unwrap(),
            );
            drop(permit);
            let _ = sender.send(());
        })
        .unwrap();
    receiver.recv_timeout(Duration::from_secs(10)).unwrap();
}

#[test]
fn completion_reports_a_recoverable_request_error_with_its_live_sql_owner() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "live-receipt", Location::Memory);
    initialize(&client);
    let first = queue(&client, "a", TransactionMode::ReadWrite);
    start(&client, "a", TransactionMode::ReadWrite, &first);
    write(&client, b"original");
    let next = queue(&client, "a", TransactionMode::ReadOnly);
    let duplicate = reply(
        &client,
        Command::Write {
            transaction: TX,
            store: "a".into(),
            key: Some(1.into()),
            value: b"duplicate".to_vec().into(),
            indexes: vec![],
            add_only: true,
            quota: None,
        },
    );
    assert!(matches!(
        duplicate.result,
        Err(IndexedDbError::Constraint(_))
    ));
    assert_eq!(duplicate.active_transaction, Some(TX));
    assert_eq!(first.handle().status(), TransactionRequestStatus::Started);
    assert!(!next.handle().is_ready());
    let committed = reply(
        &client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    );
    assert!(committed.result.is_ok());
    assert_eq!(committed.active_transaction, None);
    assert_eq!(first.handle().status(), TransactionRequestStatus::Finished);
    assert!(next.handle().is_ready());
}

#[test]
fn lost_read_transaction_clears_the_rust_owner_before_releasing_admission() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "lost-read-owner", Location::Memory);
    initialize(&client);
    let first = queue(&client, "a", TransactionMode::ReadWrite);
    start(&client, "a", TransactionMode::ReadWrite, &first);
    write(&client, b"must roll back");
    let next = queue(&client, "a", TransactionMode::ReadWrite);
    // Simulate an automatic SQLite rollback outside our write SAVEPOINT path:
    // the native connection is autocommit, while Rust still has ActiveTransaction.
    with_database(&client, |database| {
        database.connection.execute_batch("ROLLBACK").unwrap();
    });
    let failure = reply(
        &client,
        Command::Count {
            transaction: TX,
            store: "a".into(),
            source: Default::default(),
            range: Default::default(),
        },
    );
    assert!(matches!(
        failure.result,
        Err(IndexedDbError::TransactionInactive(_))
    ));
    assert_eq!(failure.active_transaction, None);
    assert_eq!(first.handle().status(), TransactionRequestStatus::Finished);
    assert!(next.handle().is_ready());
    start(&client, "a", TransactionMode::ReadWrite, &next);
    let value = reply(
        &client,
        Command::Write {
            transaction: TX,
            store: "a".into(),
            key: None,
            value: b"new".to_vec().into(),
            indexes: vec![],
            add_only: false,
            quota: None,
        },
    );
    assert!(matches!(value.result, Ok(CommandResult::Key(key)) if key == 1.into()));
    assert_eq!(value.active_transaction, Some(TX));
    execute(&client, Command::Abort { transaction: TX }).unwrap();
}

#[test]
fn automatic_whole_transaction_rollback_has_a_terminal_receipt() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "rollback-receipt", Location::Memory);
    initialize(&client);
    with_database(&client, |database| {
        database.connection.execute_batch("CREATE TEMP TRIGGER forced_rollback BEFORE DELETE ON main.records BEGIN SELECT RAISE(ROLLBACK, 'injected rollback'); END").unwrap();
    });
    let first = queue(&client, "a", TransactionMode::ReadWrite);
    start(&client, "a", TransactionMode::ReadWrite, &first);
    write(&client, b"first request");
    let next = queue(&client, "a", TransactionMode::ReadOnly);
    let failure = reply(
        &client,
        Command::Delete {
            transaction: TX,
            store: "a".into(),
            range: Default::default(),
        },
    );
    assert!(matches!(
        failure.result,
        Err(IndexedDbError::TransactionInactive(_))
    ));
    assert_eq!(failure.active_transaction, None);
    assert!(next.handle().is_ready());
    start(&client, "a", TransactionMode::ReadOnly, &next);
    assert!(matches!(
        execute(
            &client,
            Command::Count {
                transaction: TX,
                store: "a".into(),
                source: Default::default(),
                range: Default::default()
            }
        )
        .unwrap(),
        CommandResult::Count(0)
    ));
    execute(
        &client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
}

#[test]
fn orphan_sql_begin_is_rolled_back_before_a_new_owner_can_start() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "orphan-owner", Location::Memory);
    initialize(&client);
    with_database(&client, |database| {
        database
            .connection
            .execute_batch("BEGIN IMMEDIATE")
            .unwrap();
    });
    let failure = reply(
        &client,
        Command::Count {
            transaction: TX,
            store: "a".into(),
            source: Default::default(),
            range: Default::default(),
        },
    );
    assert!(matches!(
        failure.result,
        Err(IndexedDbError::TransactionInactive(_))
    ));
    assert_eq!(failure.active_transaction, None);
    let next = queue(&client, "a", TransactionMode::ReadOnly);
    start(&client, "a", TransactionMode::ReadOnly, &next);
    execute(
        &client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
}

unsafe extern "C" fn deny_rollback(
    _user: *mut std::ffi::c_void,
    action: std::ffi::c_int,
    first: *const std::ffi::c_char,
    _second: *const std::ffi::c_char,
    _database: *const std::ffi::c_char,
    _trigger: *const std::ffi::c_char,
) -> std::ffi::c_int {
    if action == rusqlite::ffi::SQLITE_TRANSACTION
        && !first.is_null()
        && unsafe { std::ffi::CStr::from_ptr(first) }.to_bytes() == b"ROLLBACK"
    {
        rusqlite::ffi::SQLITE_DENY
    } else {
        rusqlite::ffi::SQLITE_OK
    }
}

#[test]
fn unowned_connection_that_cannot_roll_back_is_closed_before_its_receipt() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "unrecoverable-owner", Location::Memory);
    initialize(&client);
    with_database(&client, |database| {
        database
            .connection
            .execute_batch("BEGIN IMMEDIATE")
            .unwrap();
        assert_eq!(
            unsafe {
                rusqlite::ffi::sqlite3_set_authorizer(
                    database.connection.handle(),
                    Some(deny_rollback),
                    std::ptr::null_mut(),
                )
            },
            rusqlite::ffi::SQLITE_OK
        );
    });
    let failure = reply(
        &client,
        Command::Count {
            transaction: TX,
            store: "a".into(),
            source: Default::default(),
            range: Default::default(),
        },
    );
    assert!(failure.result.is_err());
    assert_eq!(failure.active_transaction, None);
    assert!(client.is_closed());
    assert!(matches!(
        client.submit(Command::Abort { transaction: TX }, 0, |_| {}),
        Err(DispatchError::Closed)
    ));
}

#[test]
fn stale_request_receipt_identifies_but_does_not_end_the_successor_sql_owner() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "successor-owner", Location::Memory);
    initialize(&client);
    let first = queue(&client, "a", TransactionMode::ReadWrite);
    start(&client, "a", TransactionMode::ReadWrite, &first);
    execute(&client, Command::Abort { transaction: TX }).unwrap();
    let second = queue(&client, "a", TransactionMode::ReadWrite);
    let current = TransactionHandle::from_raw(22);
    execute(
        &client,
        Command::Begin {
            transaction: current,
            mode: TransactionMode::ReadWrite,
            stores: BTreeSet::from(["a".into()]),
            version: None,
            admission: Some(second.handle().clone()),
        },
    )
    .unwrap();
    execute(
        &client,
        Command::Write {
            transaction: current,
            store: "a".into(),
            key: None,
            value: b"successor".to_vec().into(),
            indexes: vec![],
            add_only: false,
            quota: None,
        },
    )
    .unwrap();
    let third = queue(&client, "a", TransactionMode::ReadOnly);
    let stale = reply(
        &client,
        Command::Count {
            transaction: TX,
            store: "a".into(),
            source: Default::default(),
            range: Default::default(),
        },
    );
    assert!(matches!(
        stale.result,
        Err(IndexedDbError::TransactionInactive(_))
    ));
    assert_eq!(stale.active_transaction, Some(current));
    assert_eq!(second.handle().status(), TransactionRequestStatus::Started);
    assert!(!third.handle().is_ready());
    assert!(matches!(
        execute(
            &client,
            Command::Count {
                transaction: current,
                store: "a".into(),
                source: Default::default(),
                range: Default::default()
            }
        )
        .unwrap(),
        CommandResult::Count(1)
    ));
    execute(
        &client,
        Command::Commit {
            transaction: current,
            quota: None,
        },
    )
    .unwrap();
    assert!(third.handle().is_ready());
}
