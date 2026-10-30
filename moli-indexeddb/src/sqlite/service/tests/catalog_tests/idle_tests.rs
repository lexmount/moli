use super::*;

fn physical_connections(service: &Service) -> usize {
    service
        .registry
        .lock()
        .entries
        .values()
        .map(|client| usize::from(physical_connection(client)))
        .sum()
}

fn close(connection: &DatabaseConnection) -> Result<(), IndexedDbError> {
    let (sender, receiver) = mpsc::channel();
    connection
        .close(move |reply| {
            let _ = sender.send(reply);
        })
        .unwrap();
    receiver.recv_timeout(Duration::from_secs(10)).unwrap()
}

#[cfg(target_os = "linux")]
fn open_files(root: &std::path::Path) -> usize {
    fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(|entry| fs::read_link(entry.ok()?.path()).ok())
        .filter(|path| path.starts_with(root))
        .count()
}

#[test]
fn disk_reclaims_only_after_the_last_web_connection_and_transaction_finish() {
    let directory = Directory::new();
    let service = Service::on_disk(directory.0.join("idb-lite"), config()).unwrap();
    let client = resolve(&service, OWNER, "idle");
    assert_eq!(physical_connections(&service), 1);
    let first = client.open_connection_for_test(Arc::new(|| {})).unwrap();
    seed(&client);
    let second = client.open_connection_for_test(Arc::new(|| {})).unwrap();
    let reader = second
        .queue_transaction_start(&["a".into()], TransactionMode::ReadOnly, Arc::new(|| {}))
        .unwrap();
    start(&client, "a", TransactionMode::ReadOnly, &reader);
    close(&first).unwrap();
    reap_idle(&service);
    assert_eq!(physical_connections(&service), 1);
    assert!(
        close(&second).is_err(),
        "an accepted reader still owns this connection"
    );
    assert_eq!(query(&client)[0].value, Some(b"keep".to_vec().into()));
    execute(
        &client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
    reap_idle(&service);
    assert_eq!(physical_connections(&service), 1);
    close(&second).unwrap();
    assert_eq!(
        physical_connections(&service),
        1,
        "closing a Web connection starts the grace period"
    );
    maintain_all(&service, Instant::now());
    assert_eq!(physical_connections(&service), 1);
    reap_idle(&service);
    assert_eq!(physical_connections(&service), 0);
    assert!(
        !client.is_closed(),
        "physical reclamation preserves the logical database"
    );
    #[cfg(target_os = "linux")]
    assert_eq!(open_files(&directory.0), 0);

    let metadata = client.committed().unwrap();
    let reopened = client.open_connection_for_test(Arc::new(|| {})).unwrap();
    verify_record(&client);
    assert!(Arc::ptr_eq(&metadata, &client.committed().unwrap()));
    assert_eq!(physical_connections(&service), 1);
    close(&reopened).unwrap();
    reap_idle(&service);
    assert_eq!(physical_connections(&service), 0);
}

#[test]
fn memory_records_survive_the_last_web_connection_closing() {
    let service = Service::new(config()).unwrap();
    let client = resolve(&service, OWNER, "memory-idle");
    let first = client.open_connection_for_test(Arc::new(|| {})).unwrap();
    seed(&client);
    close(&first).unwrap();
    reap_idle(&service);
    assert_eq!(physical_connections(&service), 1);
    let second = client.open_connection_for_test(Arc::new(|| {})).unwrap();
    verify_record(&client);
    close(&second).unwrap();
    assert_eq!(physical_connections(&service), 1);
}

#[test]
fn catalog_recovers_eighty_databases_without_retaining_physical_connections() {
    let directory = Directory::new();
    let root = directory.0.join("idb-lite");
    let config = config();
    let mut service = Service::on_disk(root.clone(), config).unwrap();
    for index in 0..80 {
        let client = resolve(&service, OWNER, &format!("history-{index}"));
        seed(&client);
        // Deterministic expiry lets this same fixture run under a low FD
        // limit while creating the historical profile, before testing recovery.
        reap_idle(&service);
    }
    assert_eq!(physical_connections(&service), 0);
    let usage = service.owner_usage_bytes(OWNER).unwrap();
    service.shutdown().unwrap();
    let recovered = Service::on_disk(root, config).unwrap();
    assert_eq!(recovered.databases(OWNER).unwrap().len(), 80);
    assert_eq!(recovered.owner_usage_bytes(OWNER).unwrap(), usage);
    assert_eq!(physical_connections(&recovered), 0);
    #[cfg(target_os = "linux")]
    assert_eq!(open_files(&directory.0), 0);
    let client = resolve(&recovered, OWNER, "history-37");
    assert_eq!(physical_connections(&recovered), 0);
    let connection = client.open_connection_for_test(Arc::new(|| {})).unwrap();
    verify_record(&client);
    assert_eq!(physical_connections(&recovered), 1);
    close(&connection).unwrap();
    reap_idle(&recovered);
    assert_eq!(physical_connections(&recovered), 0);
    #[cfg(target_os = "linux")]
    assert_eq!(open_files(&directory.0), 0);
}

fn install_connection_marker(client: &DatabaseClient) {
    with_database(client, |database| {
        database
            .connection
            .execute_batch(
                "CREATE TEMP TABLE connection_marker(n); INSERT INTO connection_marker VALUES(42)",
            )
            .unwrap();
    });
}

fn check_connection_marker(client: &DatabaseClient) {
    with_database(client, |database| {
        let value: i64 = database
            .connection
            .query_row("SELECT n FROM connection_marker", [], |row| row.get(0))
            .unwrap();
        assert_eq!(value, 42);
    });
}

#[test]
fn disk_reopen_within_grace_reuses_the_connection_and_expiry_reopens_it() {
    let directory = Directory::new();
    let service = Service::on_disk(directory.0.join("idb-lite"), config()).unwrap();
    let client = resolve(&service, OWNER, "reuse");
    let connection = client.open_connection_for_test(Arc::new(|| {})).unwrap();
    seed(&client);
    install_connection_marker(&client);
    close(&connection).unwrap();
    maintain_all(&service, Instant::now());
    let again = resolve(&service, OWNER, "reuse");
    let connection = again.open_connection_for_test(Arc::new(|| {})).unwrap();
    check_connection_marker(&again);
    close(&connection).unwrap();
    reap_idle(&service);
    assert!(!physical_connection(&again));
    // Cached metadata does not reopen the physical file just to read schema.
    let again = resolve(&service, OWNER, "reuse");
    assert!(!physical_connection(&again));
    let connection = again.open_connection_for_test(Arc::new(|| {})).unwrap();
    verify_record(&again);
    with_database(&again, |database| {
        assert!(
            database
                .connection
                .prepare("SELECT n FROM connection_marker")
                .is_err()
        );
    });
    close(&connection).unwrap();
}

#[test]
fn pending_inline_key_savepoint_prevents_idle_reclamation_without_a_web_connection() {
    let directory = Directory::new();
    let service = Service::on_disk(directory.0.join("idb-lite"), config()).unwrap();
    let client = resolve(&service, OWNER, "inline");
    initialize(&client);
    let lease = queue(&client, "a", TransactionMode::ReadWrite);
    start(&client, "a", TransactionMode::ReadWrite, &lease);
    let CommandResult::Prepared(prepared) = execute(
        &client,
        Command::PrepareWrite {
            transaction: TX,
            store: "a".into(),
            add_only: false,
        },
    )
    .unwrap() else {
        panic!("prepared inline key");
    };
    reap_idle(&service);
    assert!(physical_connection(&client));
    execute(
        &client,
        Command::CancelWrite {
            transaction: TX,
            request: prepared.id,
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
    reap_idle(&service);
    assert!(!physical_connection(&client));
}

#[test]
fn opening_head_protects_its_connection_and_cancellation_allows_later_reclamation() {
    let directory = Directory::new();
    let service = Service::on_disk(directory.0.join("idb-lite"), config()).unwrap();
    let head = service
        .queue_connection_request(OWNER, "opening", Arc::new(|| {}))
        .unwrap();
    let client = resolve(&service, OWNER, "opening");
    install_connection_marker(&client);
    let now = Instant::now() + Duration::from_secs(10);
    maintain_all(&service, now);
    assert!(physical_connection(&client));
    drop(head);
    // The next observation starts grace for a canceled open; no second
    // opening counter or lifecycle ticket needs to be released.
    maintain_all(&service, now);
    assert!(physical_connection(&client));
    maintain_all(&service, now + maintenance::IDLE_GRACE);
    assert!(!physical_connection(&client));
}

#[test]
fn queued_maintenance_observes_a_new_factory_head_before_closing() {
    let directory = Directory::new();
    let service = Service::on_disk(directory.0.join("idb-lite"), config()).unwrap();
    let client = resolve(&service, OWNER, "queued-check");
    install_connection_marker(&client);
    let held = hold_database(&client);
    let receipt = client
        .owner
        .maintain(Instant::now() + Duration::from_secs(10))
        .unwrap();
    let head = service
        .queue_connection_request(OWNER, "queued-check", Arc::new(|| {}))
        .unwrap();
    let (sender, receiver) = mpsc::channel();
    client
        .open_connection(Arc::new(|| {}), move |reply| {
            let _ = sender.send(reply);
        })
        .unwrap();
    drop(held);
    service
        .client
        .runtime
        .wait(async move {
            receipt.await.unwrap().unwrap();
        })
        .unwrap();
    let CommandResult::Connection(connection) = receive(&receiver).result.unwrap() else {
        panic!("connection receipt");
    };
    drop(head);
    check_connection_marker(&client);
    close(&connection).unwrap();
}

#[test]
fn maintenance_submits_all_databases_before_waiting_for_a_busy_one() {
    let directory = Directory::new();
    let service = Service::on_disk(directory.0.join("idb-lite"), config()).unwrap();
    let slow = resolve(&service, OWNER, "a-slow");
    let fast = resolve(&service, OWNER, "b-fast");
    let held = hold_database(&slow);
    let mut receipts = maintenance::PendingChecks::new();
    maintenance::submit_round(
        &service.registry,
        Instant::now() + Duration::from_secs(10),
        &mut receipts,
    );
    assert_eq!(receipts.len(), 2);
    assert!(
        !physical_connection(&fast),
        "fast reclamation must not wait for slow SQL"
    );
    drop(held);
    service
        .client
        .runtime
        .wait(async move {
            for receipt in receipts.into_values() {
                receipt.await.unwrap().unwrap();
            }
        })
        .unwrap();
    assert!(!physical_connection(&slow));
}

#[test]
fn shutdown_drains_a_queued_idle_check_before_releasing_the_runtime() {
    let directory = Directory::new();
    let mut service = Service::on_disk(directory.0.join("idb-lite"), config()).unwrap();
    let client = resolve(&service, OWNER, "shutdown-check");
    let held = hold_database(&client);
    let receipt = client
        .owner
        .maintain(Instant::now() + Duration::from_secs(10))
        .unwrap();
    let (sender, receiver) = mpsc::channel();
    let shutting_down = std::thread::spawn(move || {
        let _ = sender.send(service.shutdown());
    });
    assert!(matches!(
        receiver.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    drop(held);
    receiver
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap();
    shutting_down.join().unwrap();
    assert!(client.is_retired());
    // Even an unconsumed maintenance receipt does not hold up final close.
    assert!(receipt.blocking_recv().unwrap().is_ok());
    #[cfg(target_os = "linux")]
    assert_eq!(open_files(&directory.0), 0);
}

#[test]
fn maintenance_progresses_later_rounds_while_another_database_is_blocked() {
    let directory = Directory::new();
    let service = Service::on_disk(directory.0.join("idb-lite"), config()).unwrap();
    let slow = resolve(&service, OWNER, "a-held");
    let held = hold_database(&slow);
    let deadline = Instant::now() + Duration::from_secs(5);
    while slow.owner.maintenance_submitted() == 0 {
        assert!(
            Instant::now() < deadline,
            "first maintenance round did not start"
        );
        std::thread::yield_now();
    }
    // This database did not exist in the blocked round. Its reclamation must
    // come from a subsequent real timer round, with no manual clock advance.
    let fast = resolve(&service, OWNER, "b-later");
    let deadline = Instant::now() + Duration::from_secs(6);
    while physical_connection(&fast) {
        assert!(
            Instant::now() < deadline,
            "a blocked owner stalled later maintenance rounds"
        );
        std::thread::yield_now();
    }
    assert_eq!(
        slow.owner.maintenance_submitted(),
        1,
        "only one check may wait behind held SQL"
    );
    drop(held);
}
