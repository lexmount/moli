use super::*;
use std::sync::atomic::AtomicUsize;

const OWNER: &[u8] = b"opaque\0bucket/one";

fn resolve(service: &Service, owner: &[u8], name: &str) -> DatabaseClient {
    let (sender, receiver) = mpsc::channel();
    let client = service
        .resolve(owner.to_vec(), name.into(), move |reply| {
            let _ = sender.send(reply);
        })
        .unwrap();
    receive(&receiver).result.unwrap();
    client
}

fn seed(client: &DatabaseClient) {
    initialize(client);
    let lease = queue(client, "a", TransactionMode::ReadWrite);
    start(client, "a", TransactionMode::ReadWrite, &lease);
    write(client, b"committed");
    execute(
        client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
}

#[test]
fn reserved_owner_clear_uses_its_only_control_slot_without_fencing_early() {
    let mut settings = config();
    settings.executor.control.tasks = 1;
    let service = Service::new(settings).unwrap();
    let client = resolve(&service, OWNER, "reserved");
    seed(&client);
    let request = service.reserve_clear_owner(OWNER.to_vec()).unwrap();
    assert_eq!(service.queue_usage().control_tasks, 1);
    assert!(!client.is_closed());
    assert_eq!(service.databases(OWNER).unwrap().len(), 1);
    assert!(matches!(
        service.reserve_clear_owner(OWNER.to_vec()),
        Err(IndexedDbError::Resource(_))
    ));
    request.execute().unwrap();
    assert!(client.is_retired());
    assert_eq!(service.queue_usage().control_tasks, 0);
    assert!(service.databases(OWNER).unwrap().is_empty());
}

#[test]
fn duplicate_owner_clear_reservations_can_be_dropped_without_mutating_the_owner() {
    let service = Service::new(config()).unwrap();
    let client = resolve(&service, OWNER, "duplicates");
    seed(&client);
    let baseline = service.metadata_bytes();
    let first = service.reserve_clear_owner(OWNER.to_vec()).unwrap();
    let duplicate = service.reserve_clear_owner(OWNER.to_vec()).unwrap();
    assert_eq!(service.queue_usage().control_tasks, 2);
    assert!(!client.is_closed());
    assert_eq!(service.databases(OWNER).unwrap().len(), 1);
    drop((first, duplicate));
    assert_eq!(service.queue_usage().control_tasks, 0);
    assert_eq!(service.metadata_bytes(), baseline);
    assert!(!client.is_closed());
    service
        .reserve_clear_owner(OWNER.to_vec())
        .unwrap()
        .execute()
        .unwrap();
    assert!(client.is_retired());
}

#[test]
fn owner_clear_fences_old_clients_and_fifo_until_all_workers_confirm_cleanup() {
    let mut settings = config();
    settings.executor.workers = 2;
    let service = Service::new(settings).unwrap();
    let mut clients = Vec::new();
    let mut connections = Vec::new();
    let mut admissions = Vec::new();
    for name in ["a", "b", "c", "d"] {
        let client = resolve(&service, OWNER, name);
        seed(&client);
        let connection = client.open_connection(Arc::new(|| {})).unwrap();
        let lease = connection
            .queue_transaction_start(&["a".into()], TransactionMode::ReadWrite, Arc::new(|| {}))
            .unwrap();
        connections.push(connection);
        start(&client, "a", TransactionMode::ReadWrite, &lease);
        write(&client, b"uncommitted");
        admissions.push(lease);
        clients.push(client);
    }
    let wakes = Arc::new(AtomicUsize::new(0));
    let wake: crate::ConnectionRequestWake = {
        let wakes = wakes.clone();
        Arc::new(move || {
            wakes.fetch_add(1, Ordering::Relaxed);
        })
    };
    let head = service
        .queue_connection_request(OWNER, "unregistered", wake.clone())
        .unwrap();
    let next = service
        .queue_connection_request(OWNER, "unregistered", wake)
        .unwrap();
    let foreign = service
        .queue_connection_request(b"opaque\0bucket/one-more", "unregistered", Arc::new(|| {}))
        .unwrap();
    let data = clients[0]
        .try_reserve(TaskKind::Data, settings.executor.data.bytes)
        .unwrap();
    let held_worker = service.lanes[0].lock();
    let clear = service.prepare_clear_owner(OWNER.to_vec()).unwrap();
    assert!(clients.iter().all(DatabaseClient::is_closed));
    assert!(clients.iter().all(|client| !client.is_retired()));
    assert!(
        service
            .resolve(OWNER.to_vec(), "new".into(), |_| {})
            .is_err()
    );
    assert!(
        service
            .open(clients[0].identity().clone(), Location::Memory, |_| {})
            .is_err()
    );
    assert!(service.databases(OWNER).is_err());
    assert!(service.owner_usage_bytes(OWNER).is_err());
    assert!(service.owners_with_prefix_usage_bytes(b"opaque").is_err());
    assert!(
        service
            .queue_connection_request(OWNER, "new", Arc::new(|| {}))
            .is_err()
    );
    assert!(head.handle().is_active() && next.handle().is_active());
    assert!(
        connections
            .iter()
            .all(|connection| !connection.is_forced_closed())
    );
    let (sender, receiver) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _ = sender.send(clear.execute());
    });
    assert!(matches!(
        receiver.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    drop(held_worker);
    receiver
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap();
    worker.join().unwrap();
    assert!(clients.iter().all(DatabaseClient::is_retired));
    assert!(connections.iter().all(DatabaseConnection::is_forced_closed));
    assert_eq!(
        service.queue_usage().control_tasks,
        0,
        "retained closed wrappers must not pin retirement capacity"
    );
    assert!(
        admissions
            .iter()
            .all(|lease| lease.handle().status() == TransactionRequestStatus::Canceled)
    );
    assert!(!head.handle().is_active() && !next.handle().is_active());
    assert!(foreign.handle().is_active());
    assert_eq!(wakes.load(Ordering::Relaxed), 2);
    assert!(service.databases(OWNER).unwrap().is_empty());
    assert_eq!(service.owner_usage_bytes(OWNER).unwrap(), 0);
    drop(data);
    let fresh = resolve(&service, OWNER, "a");
    assert_ne!(
        fresh.identity().generation,
        clients[0].identity().generation
    );
    assert_eq!(fresh.committed().unwrap().version, 0);
    assert!(
        clients[0]
            .submit(Command::Abort { transaction: TX }, 0, |_| {})
            .is_err()
    );
}

#[test]
fn owner_clear_rejects_a_sql_worker_before_waiting_on_itself() {
    let service = Service::new(config()).unwrap();
    let client = resolve(&service, OWNER, "worker-reentry");
    seed(&client);
    let controller = service.client();
    let (sender, receiver) = mpsc::channel();
    client
        .try_reserve(TaskKind::Data, 0)
        .unwrap()
        .submit(client.id.0, move |_| {
            let _ = sender.send(controller.prepare_clear_owner(OWNER.to_vec()).map(|_| ()));
        })
        .unwrap();
    assert!(matches!(
        receiver.recv_timeout(Duration::from_secs(10)).unwrap(),
        Err(IndexedDbError::InvalidState(_))
    ));
    assert!(!client.is_closed());
    assert_eq!(service.databases(OWNER).unwrap().len(), 1);
    let request = service.reserve_clear_owner(OWNER.to_vec()).unwrap();
    let (sender, receiver) = mpsc::channel();
    client
        .executor
        .try_reserve(TaskKind::Data, 0)
        .unwrap()
        .submit(client.id.0, move |_| {
            let _ = sender.send(request.execute());
        })
        .unwrap();
    assert!(matches!(
        receiver.recv_timeout(Duration::from_secs(10)).unwrap(),
        Err(IndexedDbError::InvalidState(_))
    ));
    assert!(
        !client.is_closed(),
        "deferred clear must reject worker reentry before installing its fence"
    );
    let clear = service.prepare_clear_owner(OWNER.to_vec()).unwrap();
    let (sender, receiver) = mpsc::channel();
    client
        .executor
        .try_reserve(TaskKind::Data, 0)
        .unwrap()
        .submit(client.id.0, move |_| {
            let _ = sender.send(clear.execute());
        })
        .unwrap();
    assert!(matches!(
        receiver.recv_timeout(Duration::from_secs(10)).unwrap(),
        Err(IndexedDbError::InvalidState(_))
    ));
    // Its failed fence can be retried from a genuine storage owner thread.
    service
        .prepare_clear_owner(OWNER.to_vec())
        .unwrap()
        .execute()
        .unwrap();
}

#[test]
fn canceled_clear_keeps_its_fence_until_a_retry_finishes() {
    let service = Service::new(config()).unwrap();
    let client = resolve(&service, OWNER, "kept-until-retry");
    seed(&client);
    let lease = service
        .queue_connection_request(OWNER, "unknown", Arc::new(|| {}))
        .unwrap();
    let before = service.metadata_bytes();
    drop(service.prepare_clear_owner(OWNER.to_vec()).unwrap());
    assert!(service.metadata_bytes() > before);
    assert!(!client.is_retired());
    assert!(lease.handle().is_active());
    assert!(
        service
            .resolve(OWNER.to_vec(), "new".into(), |_| {})
            .is_err()
    );
    service
        .prepare_clear_owner(OWNER.to_vec())
        .unwrap()
        .execute()
        .unwrap();
    assert!(client.is_retired());
    assert!(!lease.handle().is_active());
    assert!(service.databases(OWNER).unwrap().is_empty());
}

#[test]
fn clearing_an_unregistered_owner_cancels_fifo_without_creating_a_database() {
    let directory = Directory::new();
    let root = directory.0.join("idb-lite");
    let service = Service::on_disk(root.clone(), config()).unwrap();
    let first = service
        .queue_connection_request(OWNER, "unknown", Arc::new(|| {}))
        .unwrap();
    let second = service
        .queue_connection_request(OWNER, "unknown", Arc::new(|| {}))
        .unwrap();
    service
        .prepare_clear_owner(OWNER.to_vec())
        .unwrap()
        .execute()
        .unwrap();
    assert!(!first.handle().is_active() && !second.handle().is_active());
    assert!(service.databases(OWNER).unwrap().is_empty());
    assert_eq!(std::fs::read_dir(root.join("live")).unwrap().count(), 0);
    let fresh = service
        .queue_connection_request(OWNER, "unknown", Arc::new(|| {}))
        .unwrap();
    assert!(fresh.handle().is_active() && fresh.handle().is_head());
    assert_ne!(fresh.handle().id(), first.handle().id());
}

#[test]
fn owner_clear_refuses_non_catalog_files_without_mutating_the_registration() {
    let directory = Directory::new();
    let service = Service::new(config()).unwrap();
    let (sender, receiver) = mpsc::channel();
    let client = service
        .open(
            DatabaseIdentity {
                owner_key: OWNER.to_vec(),
                ..identity("raw")
            },
            directory.location("raw.sqlite"),
            move |reply| {
                let _ = sender.send(reply);
            },
        )
        .unwrap();
    receive(&receiver).result.unwrap();
    seed(&client);
    assert!(matches!(
        service.prepare_clear_owner(OWNER.to_vec()),
        Err(IndexedDbError::InvalidState(_))
    ));
    assert!(!client.is_closed());
    assert!(directory.0.join("raw.sqlite").exists());
    assert_eq!(service.databases(OWNER).unwrap().len(), 1);
}

#[test]
fn clear_capacity_rejection_does_not_close_clients_or_install_a_fence() {
    let settings = config();
    let service = Service::new(settings).unwrap();
    let client = resolve(&service, OWNER, "capacity");
    seed(&client);
    let remaining = settings.metadata_bytes - service.metadata_bytes();
    let metadata = client.try_reserve_metadata(remaining).unwrap();
    assert!(matches!(
        service.prepare_clear_owner(OWNER.to_vec()),
        Err(IndexedDbError::Resource(_))
    ));
    assert!(!client.is_closed());
    assert!(matches!(
        service.databases(OWNER),
        Err(IndexedDbError::Resource(_))
    ));
    drop(metadata);
    assert_eq!(service.databases(OWNER).unwrap().len(), 1);
    let mut control = Vec::new();
    while let Ok(permit) = client.try_reserve(TaskKind::Control, 0) {
        control.push(permit);
    }
    assert!(matches!(
        service.prepare_clear_owner(OWNER.to_vec()),
        Err(IndexedDbError::Resource(_))
    ));
    assert!(!client.is_closed());
    drop(control);
    service
        .prepare_clear_owner(OWNER.to_vec())
        .unwrap()
        .execute()
        .unwrap();
}

#[test]
fn disk_owner_clear_retires_all_generations_and_preserves_other_owners_after_restart() {
    let directory = Directory::new();
    let root = directory.0.join("idb-lite");
    let mut settings = config();
    settings.executor.workers = 3;
    let mut service = Service::on_disk(root.clone(), settings).unwrap();
    let foreign = resolve(&service, b"foreign", "same");
    seed(&foreign);
    let mut paths = Vec::new();
    for name in ["same", "second", "creating-zero"] {
        let client = resolve(&service, OWNER, name);
        if name != "creating-zero" {
            seed(&client);
        }
        let Location::Disk { path, .. } = &client.location else {
            panic!("disk")
        };
        paths.push(path.clone());
    }
    service
        .prepare_clear_owner(OWNER.to_vec())
        .unwrap()
        .execute()
        .unwrap();
    assert!(paths.iter().all(|path| !path.exists()));
    assert_eq!(service.databases(b"foreign").unwrap().len(), 1);
    service.shutdown().unwrap();
    let service = Service::on_disk(root, settings).unwrap();
    assert!(service.databases(OWNER).unwrap().is_empty());
    let foreign = resolve(&service, b"foreign", "same");
    let lease = queue(&foreign, "a", TransactionMode::ReadOnly);
    start(&foreign, "a", TransactionMode::ReadOnly, &lease);
    assert_eq!(query(&foreign)[0].value, Some(b"committed".to_vec().into()));
    execute(
        &foreign,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
}

#[test]
fn uncertain_disk_clear_keeps_owner_fenced_and_restart_uses_durable_locations() {
    let directory = Directory::new();
    let root = directory.0.join("idb-lite");
    let mut service = Service::on_disk(root.clone(), config()).unwrap();
    let client = resolve(&service, OWNER, "uncertain");
    seed(&client);
    let generation = client.identity().generation;
    let catalog = service.catalog.as_ref().unwrap();
    catalog.interrupt_at(super::super::catalog::Boundary::Renamed);
    assert!(
        service
            .prepare_clear_owner(OWNER.to_vec())
            .unwrap()
            .execute()
            .is_err()
    );
    assert!(
        service
            .resolve(OWNER.to_vec(), "uncertain".into(), |_| {})
            .is_err()
    );
    assert!(service.owner_usage_bytes(OWNER).is_err());
    service.shutdown().unwrap();
    let service = Service::on_disk(root, config()).unwrap();
    assert!(service.databases(OWNER).unwrap().is_empty());
    let fresh = resolve(&service, OWNER, "uncertain");
    assert_ne!(fresh.identity().generation, generation);
    assert_eq!(fresh.committed().unwrap().version, 0);
}

#[test]
fn prefix_clear_fences_new_owners_and_retires_unregistered_factory_requests() {
    let mut settings = config();
    settings.executor.workers = 2;
    settings.executor.control.tasks = 1;
    let service = Service::new(settings).unwrap();
    let first = resolve(&service, b"origin\0first", "records");
    let second = resolve(&service, b"origin\0second", "records");
    let foreign = resolve(&service, b"origins\0first", "records");
    for client in [&first, &second, &foreign] {
        seed(client);
    }
    let lease = queue(&first, "a", TransactionMode::ReadWrite);
    start(&first, "a", TransactionMode::ReadWrite, &lease);
    write(&first, b"uncommitted");
    let wakes = Arc::new(AtomicUsize::new(0));
    let wake = wakes.clone();
    let unregistered = service
        .queue_connection_request(
            b"origin\0unregistered",
            "new",
            Arc::new(move || {
                wake.fetch_add(1, Ordering::Relaxed);
            }),
        )
        .unwrap();
    let foreign_request = service
        .queue_connection_request(b"origins\0unregistered", "new", Arc::new(|| {}))
        .unwrap();
    let data = first
        .try_reserve(TaskKind::Data, settings.executor.data.bytes)
        .unwrap();
    let clear = service
        .prepare_clear_owners_with_prefix(b"origin\0".to_vec())
        .unwrap();
    assert!(first.is_closed() && second.is_closed());
    assert!(!foreign.is_closed());
    assert!(
        service
            .resolve(b"origin\0never".to_vec(), "new".into(), |_| {})
            .is_err()
    );
    assert!(service.databases(b"origin\0never").is_err());
    assert!(service.owners_with_prefix_usage_bytes(b"origin").is_err());
    assert_eq!(service.queue_usage().control_tasks, 1);
    clear.execute().unwrap();
    assert!(first.is_retired() && second.is_retired());
    assert!(!unregistered.handle().is_active());
    assert!(foreign_request.handle().is_active());
    assert_eq!(wakes.load(Ordering::Relaxed), 1);
    assert_eq!(service.queue_usage().control_tasks, 0);
    drop(data);
    assert!(service.databases(b"origin\0first").unwrap().is_empty());
    assert_eq!(service.databases(b"origins\0first").unwrap().len(), 1);
    let fresh = resolve(&service, b"origin\0never", "new");
    assert_eq!(fresh.committed().unwrap().version, 0);
}

#[test]
fn broader_prefix_retry_replaces_an_abandoned_exact_fence() {
    let service = Service::new(config()).unwrap();
    let first = resolve(&service, b"origin\0first", "records");
    let second = resolve(&service, b"origin\0second", "records");
    seed(&first);
    seed(&second);
    let exact = service
        .prepare_clear_owner(b"origin\0first".to_vec())
        .unwrap();
    assert!(
        service
            .prepare_clear_owners_with_prefix(b"origin\0".to_vec())
            .is_err()
    );
    drop(exact);
    let prefix = service
        .prepare_clear_owners_with_prefix(b"origin\0".to_vec())
        .unwrap();
    assert!(
        service
            .prepare_clear_owner(b"origin\0second".to_vec())
            .is_err()
    );
    drop(prefix);
    assert!(
        service
            .resolve(b"origin\0new".to_vec(), "new".into(), |_| {})
            .is_err()
    );
    service
        .prepare_clear_owners_with_prefix(b"origin\0".to_vec())
        .unwrap()
        .execute()
        .unwrap();
    assert!(first.is_retired() && second.is_retired());
    assert!(service.databases(b"origin\0first").unwrap().is_empty());
    assert!(service.databases(b"origin\0second").unwrap().is_empty());
}

#[test]
fn prefix_clear_capacity_rejection_does_not_install_a_scope_fence() {
    let settings = config();
    let service = Service::new(settings).unwrap();
    let client = resolve(&service, b"origin\0first", "records");
    seed(&client);
    let held = service
        .try_reserve_metadata(settings.metadata_bytes - service.metadata_bytes())
        .unwrap();
    assert!(matches!(
        service.prepare_clear_owners_with_prefix(b"origin\0".to_vec()),
        Err(IndexedDbError::Resource(_))
    ));
    assert!(!client.is_closed());
    assert_eq!(service.queue_usage().control_tasks, 0);
    drop(held);
    assert_eq!(service.databases(b"origin\0first").unwrap().len(), 1);
    assert!(
        service
            .resolve(b"origin\0new".to_vec(), "new".into(), |_| {})
            .is_ok()
    );
}

#[test]
fn prefix_disk_clear_reopens_without_retired_generations_or_a_foreign_owner_loss() {
    let directory = Directory::new();
    let root = directory.0.join("idb-lite");
    let mut service = Service::on_disk(root.clone(), config()).unwrap();
    let first = resolve(&service, b"origin\0first", "records");
    let second = resolve(&service, b"origin\0second", "records");
    let foreign = resolve(&service, b"origins\0first", "records");
    for client in [&first, &second, &foreign] {
        seed(client);
    }
    service
        .prepare_clear_owners_with_prefix(b"origin\0".to_vec())
        .unwrap()
        .execute()
        .unwrap();
    service.shutdown().unwrap();
    let service = Service::on_disk(root, config()).unwrap();
    assert_eq!(
        service.owners_with_prefix_usage_bytes(b"origin\0").unwrap(),
        0
    );
    assert_eq!(service.databases(b"origins\0first").unwrap().len(), 1);
    service
        .prepare_clear_owners_with_prefix(vec![])
        .unwrap()
        .execute()
        .unwrap();
    assert!(service.databases(b"origins\0first").unwrap().is_empty());
}
