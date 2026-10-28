use super::*;

pub(super) struct WorkerGate(mpsc::Sender<()>);
impl Drop for WorkerGate {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

pub(super) fn pause_worker(service: &Service) -> WorkerGate {
    let (started, ready) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    service
        .executor
        .handle()
        .try_reserve(TaskKind::Control, 0)
        .unwrap()
        .submit(0, move |permit| {
            let _ = started.send(());
            wait.recv_timeout(Duration::from_secs(10)).unwrap();
            drop(permit);
        })
        .unwrap();
    ready.recv_timeout(Duration::from_secs(10)).unwrap();
    WorkerGate(release)
}

#[test]
fn registration_capacity_rejects_before_publishing_ids_or_creating_files() {
    let directory = Directory::new();
    let mut settings = config();
    settings.metadata_bytes = 1;
    let service = Service::new(settings).unwrap();
    assert!(matches!(
        service.open(
            identity("rejected"),
            directory.location("db.sqlite"),
            |_| panic!("no rejected callback")
        ),
        Err(DispatchError::MetadataCapacity)
    ));
    assert!(service.registry.lock().entries.is_empty());
    assert_eq!(service.registry.lock().next_id, 0);
    assert_eq!(service.metadata_bytes(), 0);
    assert_eq!(service.queue_usage().data_tasks, 0);
    assert!(!directory.0.join("db.sqlite").exists());
}

#[test]
fn existing_registration_reopens_at_full_metadata_capacity_without_copying_identity() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "same", Location::Memory);
    initialize(&client);
    let original = client.committed().unwrap();
    let identity = client.identity.clone();
    with_database(&client, move |database| {
        assert!(Arc::ptr_eq(&identity, &database.identity))
    });
    let mut held = client
        .try_reserve_metadata(config().metadata_bytes - service.metadata_bytes())
        .unwrap();
    let bytes = held.reserved_bytes();
    assert!(held.try_resize(bytes + 1).is_err());
    assert_eq!(held.reserved_bytes(), bytes);
    let (reopened, result) = register(&service, "same", Location::Memory);
    assert_eq!(reopened.id(), client.id());
    assert!(Arc::ptr_eq(&reopened.identity, &client.identity));
    let CommandResult::Metadata(snapshot) = result.result.unwrap() else {
        panic!("expected metadata")
    };
    assert!(Arc::ptr_eq(&original, &snapshot));
    assert_eq!(service.metadata_bytes(), config().metadata_bytes);
}

#[test]
fn rejected_data_admission_releases_the_unpublished_registration_charge() {
    let service = Service::new(config()).unwrap();
    let held = service
        .executor
        .handle()
        .try_reserve(TaskKind::Data, config().executor.data.bytes)
        .unwrap();
    assert!(matches!(
        service.open(identity("data-full"), Location::Memory, |_| panic!(
            "no rejected callback"
        )),
        Err(DispatchError::Executor(ExecutorError::Capacity))
    ));
    assert!(service.registry.lock().entries.is_empty());
    assert_eq!(service.metadata_bytes(), 0);
    drop(held);
    let client = open(&service, "data-full", Location::Memory);
    initialize(&client);
    assert!(service.metadata_bytes() > 0);
}

#[test]
fn registration_charge_survives_closure_until_the_last_identity_holder_is_dropped() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "old", Location::Memory);
    let held_snapshot = client.committed().unwrap();
    let charged = client.identity.reservation.reserved_bytes();
    execute(&client, Command::Close).unwrap();
    let other = open(&service, "other", Location::Memory);
    // Opening prunes the old closed registry entry; the old client still owns
    // its identity. Keeping the snapshot separates its charge from registration.
    assert_eq!(service.registry.lock().entries.len(), 1);
    let before = service.metadata_bytes();
    drop(client);
    assert_eq!(before - service.metadata_bytes(), charged);
    drop(held_snapshot);
    assert!(service.metadata_bytes() > 0);
    drop(other);
}

#[test]
fn metadata_lease_resize_and_cross_thread_drop_preserve_exact_reservations() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "lease", Location::Memory);
    let baseline = service.metadata_bytes();
    let mut lease = client.try_reserve_metadata(512).unwrap();
    assert_eq!(service.metadata_bytes(), baseline + 512);
    assert!(lease.try_resize(usize::MAX).is_err());
    assert_eq!(lease.reserved_bytes(), 512);
    assert_eq!(service.metadata_bytes(), baseline + 512);
    lease.try_resize(128).unwrap();
    assert_eq!(service.metadata_bytes(), baseline + 128);
    std::thread::spawn(move || drop(lease)).join().unwrap();
    assert_eq!(service.metadata_bytes(), baseline);
}
