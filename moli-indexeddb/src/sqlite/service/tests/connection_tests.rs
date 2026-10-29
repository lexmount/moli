use super::*;

fn noop() -> crate::VersionChangeWake {
    Arc::new(|| {})
}

fn barrier(client: &DatabaseClient) {
    let (sender, receiver) = mpsc::channel();
    client
        .submit_metadata(move |reply| {
            let _ = sender.send(reply);
        })
        .unwrap();
    receive(&receiver).result.unwrap();
}

fn close(connection: &DatabaseConnection) -> Result<(), IndexedDbError> {
    let (sender, receiver) = mpsc::channel();
    connection
        .close(move |result| {
            let _ = sender.send(result);
        })
        .unwrap();
    receiver.recv_timeout(Duration::from_secs(10)).unwrap()
}

fn start(
    connection: &DatabaseConnection,
    request: &TransactionRequestLease,
    transaction: TransactionHandle,
) {
    execute(
        connection.client(),
        Command::Begin {
            transaction,
            mode: request.handle().mode(),
            stores: request.handle().store_names().into_iter().collect(),
            version: None,
            admission: Some(request.handle().clone()),
        },
    )
    .unwrap();
}

fn put(client: &DatabaseClient, transaction: TransactionHandle) {
    execute(
        client,
        Command::Write {
            transaction,
            store: "a".into(),
            key: None,
            value: crate::IndexedDbValue::from(vec![5, 6, 7]),
            indexes: vec![],
            add_only: false,
            quota: None,
        },
    )
    .unwrap();
}

#[test]
fn logical_close_retains_accepted_transactions_and_other_connections() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "logical-close", Location::Memory);
    initialize(&client);
    let first = client.open_connection(noop()).unwrap();
    let second = client.open_connection(noop()).unwrap();
    assert_ne!(first.handle(), second.handle());
    let accepted = first
        .queue_transaction_start(&["a".into()], TransactionMode::ReadWrite, noop())
        .unwrap();
    first.mark_close_pending();
    assert!(first.is_closed());
    assert!(!first.is_forced_closed());
    assert!(
        first
            .queue_transaction_start(&["a".into()], TransactionMode::ReadOnly, noop())
            .is_err()
    );
    let batch = client.begin_version_change(1, Some(2), noop());
    assert!(first.take_notification().is_none());
    let notification = second.take_notification().unwrap();
    assert_eq!(batch.blocked_event_required(), None);
    drop(notification);
    assert_eq!(batch.blocked_event_required(), Some(true));
    assert!(matches!(
        close(&first),
        Err(IndexedDbError::InvalidState(_))
    ));
    let transaction = client.allocate_transaction_handle().unwrap();
    start(&first, &accepted, transaction);
    put(&client, transaction);
    assert!(close(&first).is_err());
    execute(&client, Command::Commit { transaction }).unwrap();
    close(&first).unwrap();
    assert!(!first.is_forced_closed());
    assert!(client.has_open_connections());
    let next = second
        .queue_transaction_start(&["a".into()], TransactionMode::ReadOnly, noop())
        .unwrap();
    let transaction = client.allocate_transaction_handle().unwrap();
    start(&second, &next, transaction);
    assert!(matches!(
        execute(
            &client,
            Command::Count {
                transaction,
                store: "a".into(),
                source: QuerySource::ObjectStore,
                range: crate::KeyRange::default()
            }
        )
        .unwrap(),
        CommandResult::Count(1)
    ));
    execute(&client, Command::Commit { transaction }).unwrap();
    close(&second).unwrap();
    assert!(!client.has_open_connections());
    assert!(!client.is_closed());
    assert_eq!(client.committed().unwrap().version, 1);
}

#[test]
fn dropping_an_owner_rolls_back_before_canceling_admission_and_notifying_blockers() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "owner-drop", Location::Memory);
    initialize(&client);
    let first = client.open_connection(noop()).unwrap();
    let signal = first.close_signal();
    let second = client.open_connection(noop()).unwrap();
    let request = first
        .queue_transaction_start(&["a".into()], TransactionMode::ReadWrite, noop())
        .unwrap();
    let transaction = client.allocate_transaction_handle().unwrap();
    start(&first, &request, transaction);
    put(&client, transaction);
    let canceled = first
        .queue_transaction_start(&["b".into()], TransactionMode::ReadOnly, noop())
        .unwrap();
    let (sender, receiver) = mpsc::channel();
    let next = second
        .queue_transaction_start(&["a".into()], TransactionMode::ReadOnly, {
            let signal = signal.clone();
            Arc::new(move || {
                let _ = sender.send(signal.is_forced_closed());
            })
        })
        .unwrap();
    let batch = client.begin_version_change(1, Some(2), noop());
    let notification = first.take_notification().unwrap();
    drop(second.take_notification());
    drop(first);
    assert!(receiver.recv_timeout(Duration::from_secs(10)).unwrap());
    barrier(&client);
    assert_eq!(
        request.handle().status(),
        TransactionRequestStatus::Canceled
    );
    assert_eq!(
        canceled.handle().status(),
        TransactionRequestStatus::Canceled
    );
    assert!(next.handle().is_ready());
    assert!(signal.is_forced_closed());
    assert_eq!(
        batch.blocked_event_required(),
        None,
        "in-flight event must acknowledge its checkpoint"
    );
    drop(notification);
    assert_eq!(batch.blocked_event_required(), Some(true));
    let transaction = client.allocate_transaction_handle().unwrap();
    start(&second, &next, transaction);
    assert!(matches!(
        execute(
            &client,
            Command::Count {
                transaction,
                store: "a".into(),
                source: QuerySource::ObjectStore,
                range: crate::KeyRange::default()
            }
        )
        .unwrap(),
        CommandResult::Count(0)
    ));
    execute(&client, Command::Commit { transaction }).unwrap();
    close(&second).unwrap();
}

#[test]
fn retiring_an_idle_connection_preserves_another_connections_active_savepoint() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "other-owner", Location::Memory);
    initialize(&client);
    let first = client.open_connection(noop()).unwrap();
    let signal = first.close_signal();
    let second = client.open_connection(noop()).unwrap();
    let request = second
        .queue_transaction_start(&["a".into()], TransactionMode::ReadWrite, noop())
        .unwrap();
    let transaction = client.allocate_transaction_handle().unwrap();
    start(&second, &request, transaction);
    let prepared = execute(
        &client,
        Command::PrepareWrite {
            transaction,
            store: "a".into(),
            add_only: false,
        },
    )
    .unwrap();
    let CommandResult::Prepared(prepared) = prepared else {
        panic!("prepared write");
    };
    drop(first);
    barrier(&client);
    assert!(signal.is_forced_closed());
    execute(
        &client,
        Command::FinishWrite {
            transaction,
            request: prepared.id,
            value: crate::IndexedDbValue::from(vec![7]),
            indexes: vec![],
            quota: None,
        },
    )
    .unwrap();
    execute(&client, Command::Commit { transaction }).unwrap();
    assert!(client.committed().unwrap().usage > 0);
    close(&second).unwrap();
}

#[test]
fn upgrade_ownership_is_rolled_back_when_its_connection_is_lost() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "upgrade-owner", Location::Memory);
    let connection = client.open_connection(noop()).unwrap();
    let signal = connection.close_signal();
    let transaction = client.allocate_transaction_handle().unwrap();
    let (sender, receiver) = mpsc::channel();
    connection
        .begin_upgrade(transaction, 7, move |reply| {
            let _ = sender.send(reply);
        })
        .unwrap();
    assert_eq!(receive(&receiver).active_transaction, Some(transaction));
    execute(
        &client,
        Command::CreateStore {
            transaction,
            name: "a".into(),
            options: ObjectStoreOptions::default(),
        },
    )
    .unwrap();
    assert!(close(&connection).is_err());
    drop(connection);
    barrier(&client);
    assert!(signal.is_forced_closed());
    assert!(!client.has_open_connections());
    assert_eq!(client.committed().unwrap().version, 0);
    assert!(client.committed().unwrap().stores.is_empty());
    let replacement = client.open_connection(noop()).unwrap();
    let (sender, receiver) = mpsc::channel();
    replacement
        .begin_upgrade(
            client.allocate_transaction_handle().unwrap(),
            1,
            move |reply| {
                let _ = sender.send(reply);
            },
        )
        .unwrap();
    assert!(receive(&receiver).result.is_ok());
    drop(replacement);
    barrier(&client);
    assert_eq!(client.committed().unwrap().version, 0);
}

#[test]
fn reserved_connection_cleanup_runs_with_data_and_control_capacity_exhausted() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "cleanup-capacity", Location::Memory);
    initialize(&client);
    let committed_usage = client.committed().unwrap().usage;
    let (sender, receiver) = mpsc::channel();
    let connection = client
        .open_connection(Arc::new(move || {
            let _ = sender.send(());
        }))
        .unwrap();
    let signal = connection.close_signal();
    let request = connection
        .queue_transaction_start(&["a".into()], TransactionMode::ReadWrite, noop())
        .unwrap();
    let transaction = client.allocate_transaction_handle().unwrap();
    start(&connection, &request, transaction);
    put(&client, transaction);
    let mut controls = vec![];
    while let Ok(permit) = client.try_reserve(TaskKind::Control, 512) {
        controls.push(permit);
    }
    let data = client
        .try_reserve(TaskKind::Data, config().executor.data.bytes)
        .unwrap();
    drop(connection);
    receiver.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(signal.is_forced_closed());
    assert_eq!(
        request.handle().status(),
        TransactionRequestStatus::Canceled
    );
    assert!(!client.has_open_connections());
    drop((controls, data));
    barrier(&client);
    assert_eq!(client.committed().unwrap().usage, committed_usage);
}

#[test]
fn shutdown_forces_every_connection_after_rolling_back_and_cancels_queued_work() {
    let mut service = Service::new(config()).unwrap();
    let client = open(&service, "shutdown-connections", Location::Memory);
    initialize(&client);
    let connection = client.open_connection(noop()).unwrap();
    let request = connection
        .queue_transaction_start(&["a".into()], TransactionMode::ReadWrite, noop())
        .unwrap();
    let transaction = client.allocate_transaction_handle().unwrap();
    start(&connection, &request, transaction);
    put(&client, transaction);
    let pending = connection
        .queue_transaction_start(&["b".into()], TransactionMode::ReadWrite, noop())
        .unwrap();
    let batch = client.begin_version_change(1, None, noop());
    service.shutdown().unwrap();
    assert!(connection.is_forced_closed());
    assert!(!client.has_open_connections());
    assert_eq!(batch.blocked_event_required(), Some(false));
    assert_eq!(
        request.handle().status(),
        TransactionRequestStatus::Canceled
    );
    assert_eq!(
        pending.handle().status(),
        TransactionRequestStatus::Canceled
    );
    assert!(
        connection
            .queue_transaction_start(&["a".into()], TransactionMode::ReadOnly, noop())
            .is_err()
    );
}

#[test]
fn connection_registration_failure_does_not_publish_a_blocker_or_consume_metadata() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "registration-pressure", Location::Memory);
    let before = service.metadata_bytes();
    let mut controls = vec![];
    while let Ok(permit) = client.try_reserve(TaskKind::Control, 512) {
        controls.push(permit);
    }
    assert!(matches!(
        client.open_connection(noop()),
        Err(IndexedDbError::Resource(_))
    ));
    assert_eq!(service.metadata_bytes(), before);
    assert!(!client.has_open_connections());
    drop(controls);
    let connection = client.open_connection(noop()).unwrap();
    let other_client = open(&service, "other-registration", Location::Memory);
    let other = other_client.open_connection(noop()).unwrap();
    assert_ne!(
        connection.handle(),
        other.handle(),
        "handles must be unique across logical databases"
    );
    let retained = service.metadata_bytes();
    close(&connection).unwrap();
    assert_eq!(
        service.metadata_bytes(),
        retained,
        "surviving lease retains its allocation budget"
    );
    drop(connection);
    barrier(&client);
    assert!(service.metadata_bytes() < retained);
    close(&other).unwrap();
}

#[test]
fn connection_request_fifo_is_partition_shared_bounded_and_cancelable() {
    let service = Service::new(ServiceConfig {
        admissions: 3,
        ..config()
    })
    .unwrap();
    let first = service
        .queue_connection_request(b"\0a", "db", noop())
        .unwrap();
    let (sender, receiver) = mpsc::channel();
    let second = service
        .queue_connection_request(
            b"\0a",
            "db",
            Arc::new(move || {
                let _ = sender.send(());
            }),
        )
        .unwrap();
    let other = service
        .queue_connection_request(b"a\0", "db", noop())
        .unwrap();
    assert!(first.handle().is_head());
    assert!(!second.handle().is_head());
    assert!(other.handle().is_head());
    assert!(matches!(
        service.queue_connection_request(b"owner", "db", noop()),
        Err(IndexedDbError::Resource(_))
    ));
    std::thread::spawn(move || drop(first)).join().unwrap();
    receiver.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(second.handle().is_head());
    let reused = service
        .queue_connection_request(b"owner", "db", noop())
        .unwrap();
    drop((second, other, reused));
    let tiny = Service::new(ServiceConfig {
        admission_bytes: 1,
        ..config()
    })
    .unwrap();
    assert!(matches!(
        tiny.queue_connection_request(b"owner", "db", noop()),
        Err(IndexedDbError::Resource(_))
    ));
}
