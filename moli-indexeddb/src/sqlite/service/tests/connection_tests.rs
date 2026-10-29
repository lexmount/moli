use super::*;

fn noop() -> crate::VersionChangeWake {
    Arc::new(|| {})
}

#[test]
fn versionchange_checkpoint_waits_for_an_already_requested_native_close() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "checkpoint-close", Location::Memory);
    initialize(&client);
    let connection = client.open_connection(noop()).unwrap();
    let batch = client.begin_version_change(1, Some(2), noop());
    let notification = connection.take_notification().unwrap();
    // Hold the actual SQL lane to make the close receipt arrive strictly after
    // the recipient's event checkpoint, without depending on thread speed.
    let worker = client.lane.lock();
    let (sender, receiver) = mpsc::channel();
    connection
        .close(move |result| {
            let _ = sender.send(result);
        })
        .unwrap();
    drop(notification.completion);
    let at_checkpoint = batch.blocked_event_required();
    drop(worker);
    receiver
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap();
    assert_eq!(
        at_checkpoint, None,
        "an already-requested close needs its native receipt before deciding blocked"
    );
    assert_eq!(batch.blocked_event_required(), Some(false));
}

#[test]
fn versionchange_checkpoint_waits_until_the_closed_connection_is_unregistered() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "unregistration-checkpoint", Location::Memory);
    initialize(&client);
    let connection = client.open_connection(noop()).unwrap();
    let batch = client.begin_version_change(1, Some(2), noop());
    let notification = connection.take_notification().unwrap();
    let (entered, waiting) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    client
        .state
        .notifications
        .before_next_unregistration(move || {
            entered.send(()).unwrap();
            resume.recv_timeout(Duration::from_secs(10)).unwrap();
        });
    let (sender, receiver) = mpsc::channel();
    connection
        .close(move |result| {
            let _ = sender.send(result);
        })
        .unwrap();
    waiting.recv_timeout(Duration::from_secs(10)).unwrap();
    // SQL close has run, but the old notification registration is still live.
    // Its event checkpoint must wait rather than commit a stale blocked=true.
    drop(notification.completion);
    let at_checkpoint = batch.blocked_event_required();
    release.send(()).unwrap();
    receiver
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap();
    assert_eq!(at_checkpoint, None);
    assert_eq!(batch.blocked_event_required(), Some(false));
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
    execute(
        &client,
        Command::Commit {
            transaction,
            quota: None,
        },
    )
    .unwrap();
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
    execute(
        &client,
        Command::Commit {
            transaction,
            quota: None,
        },
    )
    .unwrap();
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
    execute(
        &client,
        Command::Commit {
            transaction,
            quota: None,
        },
    )
    .unwrap();
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
    execute(
        &client,
        Command::Commit {
            transaction,
            quota: None,
        },
    )
    .unwrap();
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
fn connection_drop_rolls_back_before_forcing_close() {
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

    drop(connection);
    receiver.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(signal.is_forced_closed());
    assert_eq!(
        request.handle().status(),
        TransactionRequestStatus::Canceled
    );
    assert!(!client.has_open_connections());

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
fn panicked_worker_shutdown_rolls_back_and_retires_live_connections() {
    for disk in [false, true] {
        let directory = Directory::new();
        let location = if disk {
            directory.location("worker-panic.sqlite")
        } else {
            Location::Memory
        };
        let mut service = Service::new(config()).unwrap();
        let client = open(&service, "panicked-owner", location);
        initialize(&client);
        let connection = client.open_connection(noop()).unwrap();
        let request = connection
            .queue_transaction_start(&["a".into()], TransactionMode::ReadWrite, noop())
            .unwrap();
        let transaction = client.allocate_transaction_handle().unwrap();
        start(&connection, &request, transaction);
        put(&client, transaction);
        let pending = connection
            .queue_transaction_start(&["a".into()], TransactionMode::ReadOnly, noop())
            .unwrap();
        client
            .executor
            .submit(client.id.0, || {
                panic!("injected failure with a live SQL owner")
            })
            .unwrap();
        assert!(service.shutdown().is_err());
        assert!(connection.is_forced_closed());
        assert!(client.is_closed());
        assert!(!client.has_open_connections());
        for request in [&request, &pending] {
            assert_eq!(
                request.handle().status(),
                TransactionRequestStatus::Canceled
            );
        }

        if disk {
            // Retained old wrappers must no longer pin the failed SQL owner.
            let restored = Service::new(config()).unwrap();
            let next = open(
                &restored,
                "panicked-owner",
                Location::Disk {
                    path: directory.0.join("worker-panic.sqlite"),
                    mode: OpenMode::Existing,
                },
            );
            let lease = queue(&next, "a", TransactionMode::ReadOnly);
            super::start(&next, "a", TransactionMode::ReadOnly, &lease);
            assert!(query(&next).is_empty());
            execute(
                &next,
                Command::Commit {
                    transaction: TX,
                    quota: None,
                },
            )
            .unwrap();
        }
    }
}

#[test]
fn connection_registration_failure_does_not_publish_a_blocker_or_consume_a_handle() {
    let service = Service::new(config()).unwrap();
    service.next_connection.store(1u64 << 53, Ordering::Relaxed);
    let client = open(&service, "registration-pressure", Location::Memory);
    service.next_connection.store(u64::MAX, Ordering::Relaxed);
    assert!(matches!(
        client.open_connection(noop()),
        Err(IndexedDbError::Resource(_))
    ));
    assert!(!client.has_open_connections());
    assert_eq!(service.next_connection.load(Ordering::Relaxed), u64::MAX);
    service.next_connection.store(1u64 << 53, Ordering::Relaxed);

    let connection = client.open_connection(noop()).unwrap();
    assert_eq!(connection.handle().into_raw(), (1u64 << 53) + 1);

    let other_client = open(&service, "other-registration", Location::Memory);
    let other = other_client.open_connection(noop()).unwrap();
    assert_ne!(
        connection.handle(),
        other.handle(),
        "handles must be unique across logical databases"
    );
    close(&connection).unwrap();
    assert!(connection.is_close_complete());
    drop(connection);
    barrier(&client);
    assert!(other_client.has_open_connections());
    close(&other).unwrap();
}

#[test]
fn service_clients_do_not_keep_workers_alive_and_shutdown_wakes_all_factory_requests() {
    for panic_worker in [false, true] {
        let mut service = Service::new(config()).unwrap();
        let client = service.client();
        let (sender, receiver) = mpsc::channel();
        let requests = ["db", "db", "other"].map(|name| {
            let sender = sender.clone();
            client
                .queue_connection_request(
                    b"owner",
                    name,
                    Arc::new(move || {
                        let _ = sender.send(());
                    }),
                )
                .unwrap()
        });
        assert!(requests[0].handle().is_head());
        assert!(!requests[1].handle().is_head());
        if panic_worker {
            service
                .executor
                .handle()
                .submit(0, || panic!("injected worker failure before shutdown"))
                .unwrap();
            assert!(service.shutdown().is_err());
        }
        drop(service);
        assert!(client.is_closed());
        for request in requests {
            receiver.recv_timeout(Duration::from_secs(10)).unwrap();
            assert!(!request.handle().is_active());
            assert!(!request.handle().is_head());
        }
        assert!(
            client
                .queue_connection_request(b"owner", "db", noop())
                .is_err()
        );
        assert!(
            client
                .resolve(b"owner".to_vec(), "db".into(), |_| {})
                .is_err()
        );
    }
}

#[test]
fn connection_request_fifo_is_partition_shared_and_cancelable_without_a_separate_limit() {
    let service = Service::new(config()).unwrap();
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
    let independent = service
        .queue_connection_request(b"owner", "db", noop())
        .unwrap();
    assert!(first.handle().is_head());
    assert!(!second.handle().is_head());
    assert!(other.handle().is_head());
    assert!(independent.handle().is_head());
    std::thread::spawn(move || drop(first)).join().unwrap();
    receiver.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(second.handle().is_head());
    drop((second, other, independent));
    let pending = (0..1050)
        .map(|_| {
            service
                .queue_connection_request(b"owner", "queued", noop())
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert!(pending[0].handle().is_head());
    assert!(
        pending
            .iter()
            .skip(1)
            .all(|request| !request.handle().is_head())
    );
    for request in pending {
        assert!(request.handle().is_head());
        drop(request);
    }
    let large_name = "n".repeat(4 * 1024 * 1024 + 1);
    let large = service
        .queue_connection_request(b"owner", &large_name, noop())
        .unwrap();
    assert!(large.handle().is_head());
}

#[test]
fn weak_connection_does_not_keep_a_collected_owner_open() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "weak-owner", Location::Memory);
    initialize(&client);
    let connection = client.open_connection(noop()).unwrap();
    let weak = connection.downgrade();
    assert!(weak.is_alive());
    drop(connection);
    barrier(&client);
    assert!(!weak.is_alive());
    assert!(weak.upgrade().is_none());
    assert!(!client.has_open_connections());
}
