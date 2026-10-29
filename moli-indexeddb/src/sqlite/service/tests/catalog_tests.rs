use super::super::catalog::Boundary;
use super::*;
use std::fs;

mod crash_tests;

const OWNER: &[u8] = b"partition/default/top-site/storage-key/bucket-7";

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

fn path(client: &DatabaseClient) -> PathBuf {
    match &client.location {
        Location::Disk { path, .. } => path.clone(),
        Location::Memory => panic!("disk client"),
    }
}

fn seed(client: &DatabaseClient) {
    initialize(client);
    let lease = queue(client, "a", TransactionMode::ReadWrite);
    start(client, "a", TransactionMode::ReadWrite, &lease);
    write(client, b"keep");
    execute(
        client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
}

fn verify_record(client: &DatabaseClient) {
    let lease = queue(client, "a", TransactionMode::ReadOnly);
    start(client, "a", TransactionMode::ReadOnly, &lease);
    assert_eq!(query(client)[0].value, Some(b"keep".to_vec().into()));
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
fn catalog_recovers_the_largest_webidl_version_without_changing_generation_or_records() {
    let directory = Directory::new();
    let root = directory.0.join("idb-lite");
    let mut generation = None;
    for version in [1, 1_u64 << 32, (1_u64 << 53) - 1] {
        let mut service = Service::on_disk(root.clone(), config()).unwrap();
        let client = resolve(&service, OWNER, "large-version");
        match generation {
            Some(generation) => assert_eq!(client.identity().generation, generation),
            None => {
                generation = Some(client.identity().generation);
                seed(&client);
            }
        }
        if version != 1 {
            execute(
                &client,
                Command::Begin {
                    transaction: TX,
                    mode: TransactionMode::VersionChange,
                    stores: BTreeSet::new(),
                    version: Some(version),
                    admission: None,
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
        }
        assert_eq!(
            service.database_version(OWNER, "large-version").unwrap(),
            Some(version)
        );
        assert_eq!(service.databases(OWNER).unwrap()[0].version, version);
        verify_record(&client);
        service.shutdown().unwrap();
    }
    let service = Service::on_disk(root, config()).unwrap();
    let client = resolve(&service, OWNER, "large-version");
    assert_eq!(client.committed().unwrap().version, (1_u64 << 53) - 1);
    verify_record(&client);
}

#[test]
fn catalog_concurrent_resolve_reuses_one_generation_and_runs_completion_on_worker() {
    let service = Service::new(config()).unwrap();
    let caller = std::thread::current().id();
    let clients = std::thread::scope(|scope| {
        let mut threads = Vec::new();
        for _ in 0..8 {
            let service = &service;
            threads.push(scope.spawn(move || {
                let (sender, receiver) = mpsc::channel();
                let client = service
                    .resolve(OWNER.to_vec(), "same".into(), move |reply| {
                        let _ = sender.send((reply, std::thread::current().id()));
                    })
                    .unwrap();
                let (reply, worker) = receiver.recv_timeout(Duration::from_secs(10)).unwrap();
                reply.result.unwrap();
                assert_ne!(worker, caller);
                client
            }));
        }
        threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>()
    });
    for client in &clients {
        assert_eq!(client.id(), clients[0].id());
        assert_eq!(
            client.identity().generation,
            clients[0].identity().generation
        );
    }
    assert!(service.databases(OWNER).unwrap().is_empty());
    initialize(&clients[0]);
    assert_eq!(service.databases(OWNER).unwrap().len(), 1);
    assert_eq!(service.database_version(OWNER, "same").unwrap(), Some(1));
    assert_eq!(service.registry.lock().entries.len(), 1);
}

#[test]
fn catalog_lists_committed_metadata_and_snapshots_survive_upgrade_and_shutdown() {
    let mut service = Service::new(config()).unwrap();
    let astral = resolve(&service, OWNER, "\u{10000}");
    let bmp = resolve(&service, OWNER, "\u{e000}");
    let foreign = resolve(&service, b"other-bucket", "hidden");
    for client in [&astral, &bmp, &foreign] {
        initialize(client);
    }
    let snapshot = service.databases(OWNER).unwrap();
    assert_eq!(
        snapshot
            .iter()
            .map(|metadata| metadata.name.as_str())
            .collect::<Vec<_>>(),
        ["\u{10000}", "\u{e000}"]
    );
    assert!(Arc::ptr_eq(&snapshot[0], &astral.committed().unwrap()));
    execute(
        &astral,
        Command::Begin {
            transaction: TX,
            mode: TransactionMode::VersionChange,
            stores: BTreeSet::new(),
            version: Some(2),
            admission: None,
        },
    )
    .unwrap();
    assert_eq!(
        service.database_version(OWNER, "\u{10000}").unwrap(),
        Some(1)
    );
    execute(
        &astral,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
    assert_eq!(
        service.database_version(OWNER, "\u{10000}").unwrap(),
        Some(2)
    );
    assert_eq!(snapshot[0].version, 1);
    service.shutdown().unwrap();
    assert_eq!(snapshot[0].version, 1);
    assert!(service.databases(OWNER).is_err());
    assert!(service.database_version(OWNER, "\u{10000}").is_err());
    assert!(matches!(
        service.resolve(OWNER.to_vec(), "late".into(), |_| {}),
        Err(DispatchError::Closed)
    ));
}

#[test]
fn catalog_disk_restart_retains_generation_records_and_never_reads_legacy_storage() {
    let directory = Directory::new();
    let root = directory.0.join("idb-lite");
    let legacy = directory.0.join("indexeddb");
    fs::write(&legacy, b"not a directory or valid JSON").unwrap();
    let owner = [0xff, 0, 17];
    let name = "a/b\0unicode-\u{10000}";
    let mut service = Service::on_disk(root.clone(), config()).unwrap();
    let client = resolve(&service, &owner, name);
    assert_eq!(client.committed().unwrap().version, 0);
    assert!(service.databases(&owner).unwrap().is_empty());
    let file = path(&client);
    assert_eq!(file.file_name().unwrap(), "db.sqlite");
    let parts = file
        .strip_prefix(root.join("live"))
        .unwrap()
        .components()
        .collect::<Vec<_>>();
    assert_eq!(
        parts
            .iter()
            .map(|part| part.as_os_str().len())
            .collect::<Vec<_>>(),
        [64, 64, 32, 9]
    );
    assert_eq!(
        fs::read_dir(file.parent().unwrap().parent().unwrap())
            .unwrap()
            .count(),
        1
    );
    seed(&client);
    let generation = client.identity().generation;
    service.shutdown().unwrap();
    let reopened = Service::on_disk(root, config()).unwrap();
    assert_eq!(reopened.database_version(&owner, name).unwrap(), Some(1));
    let next = resolve(&reopened, &owner, name);
    assert_eq!(next.identity().generation, generation);
    verify_record(&next);
    assert_eq!(fs::read(legacy).unwrap(), b"not a directory or valid JSON");
}

#[test]
fn catalog_closed_disk_owner_reuses_known_path_even_after_registration_rejection() {
    let directory = Directory::new();
    let service = Service::on_disk(directory.0.join("idb-lite"), config()).unwrap();
    let first = resolve(&service, OWNER, "same");
    seed(&first);
    execute(&first, Command::Close).unwrap();
    let held = service
        .executor
        .handle()
        .try_reserve(config().executor.max_in_flight_bytes)
        .unwrap();
    assert!(matches!(
        service.resolve(OWNER.to_vec(), "same".into(), |_| {}),
        Err(DispatchError::Executor(ExecutorError::Capacity))
    ));
    assert_eq!(service.database_version(OWNER, "same").unwrap(), Some(1));
    drop(held);
    let next = resolve(&service, OWNER, "same");
    assert_ne!(next.id(), first.id());
    assert_eq!(next.identity().generation, first.identity().generation);
    assert_eq!(path(&next), path(&first));
    verify_record(&next);
}

#[test]
fn catalog_missing_published_file_is_an_error_and_is_never_replaced_with_an_empty_database() {
    let directory = Directory::new();
    let root = directory.0.join("idb-lite");
    let mut service = Service::on_disk(root.clone(), config()).unwrap();
    let client = resolve(&service, OWNER, "lost");
    seed(&client);
    execute(&client, Command::Close).unwrap();
    let file = path(&client);
    fs::remove_file(&file).unwrap();
    let (sender, receiver) = mpsc::channel();
    service
        .resolve(OWNER.to_vec(), "lost".into(), move |reply| {
            let _ = sender.send(reply);
        })
        .unwrap();
    assert!(receive(&receiver).result.is_err());
    assert!(service.database_version(OWNER, "lost").is_err());
    assert!(service.databases(OWNER).is_err());
    assert!(service.owner_usage_bytes(OWNER).is_err());
    assert!(matches!(
        service.resolve(OWNER.to_vec(), "lost".into(), |_| {}),
        Err(DispatchError::Storage(_))
    ));
    service.shutdown().unwrap();
    assert!(Service::on_disk(root, config()).is_err());
    assert!(file.parent().unwrap().is_dir());
    assert!(!file.exists());
}

#[test]
fn catalog_interrupted_preparation_is_recovered_before_any_database_is_published() {
    for boundary in [
        Boundary::MarkerSynced,
        Boundary::Allocated,
        Boundary::CreatingSynced,
    ] {
        let directory = Directory::new();
        let root = directory.0.join("idb-lite");
        let mut service = Service::on_disk(root.clone(), config()).unwrap();
        service.catalog.as_ref().unwrap().interrupt_at(boundary);
        let (sender, receiver) = mpsc::channel();
        let failed = service
            .resolve(OWNER.to_vec(), "new".into(), move |reply| {
                let _ = sender.send(reply);
            })
            .unwrap();
        assert!(receive(&receiver).result.is_err());
        assert!(service.database_version(OWNER, "new").is_err());
        service.shutdown().unwrap();
        let recovered = Service::on_disk(root, config()).unwrap();
        assert!(recovered.databases(OWNER).unwrap().is_empty());
        let next = resolve(&recovered, OWNER, "new");
        assert_ne!(next.identity().generation, failed.identity().generation);
        initialize(&next);
        assert_eq!(recovered.database_version(OWNER, "new").unwrap(), Some(1));
    }
}

#[test]
fn catalog_preparation_marker_cannot_discard_a_published_database() {
    let directory = Directory::new();
    let root = directory.0.join("idb-lite");
    let mut service = Service::on_disk(root.clone(), config()).unwrap();
    let client = resolve(&service, OWNER, "published");
    seed(&client);
    let file = path(&client);
    service.shutdown().unwrap();
    let generation = file.parent().unwrap();
    let name = generation.file_name().unwrap().to_str().unwrap();
    let marker = generation
        .parent()
        .unwrap()
        .join(format!(".creating-{name}"));
    fs::write(&marker, name.as_bytes()).unwrap();
    assert!(Service::on_disk(root.clone(), config()).is_err());
    assert!(file.exists());
    fs::remove_file(marker).unwrap();
    let restored = Service::on_disk(root, config()).unwrap();
    verify_record(&resolve(&restored, OWNER, "published"));
}

#[test]
fn catalog_delete_publishes_before_reply_and_same_name_gets_a_new_generation() {
    for disk in [false, true] {
        let directory = Directory::new();
        let mut service = if disk {
            Service::on_disk(directory.0.join("idb-lite"), config()).unwrap()
        } else {
            Service::new(config()).unwrap()
        };
        let old = resolve(&service, OWNER, "recreated");
        seed(&old);
        let snapshot = service.databases(OWNER).unwrap();
        let (sender, receiver) = mpsc::channel();
        old.submit(Command::DeleteDatabase, 0, move |reply| {
            let _ = sender.send(reply);
        })
        .unwrap();
        let reply = receive(&receiver);
        reply.result.unwrap();
        assert!(old.is_closed());
        assert_eq!(service.database_version(OWNER, "recreated").unwrap(), None);
        assert!(service.databases(OWNER).unwrap().is_empty());
        assert_eq!(snapshot[0].version, 1);
        if disk {
            assert!(!path(&old).parent().unwrap().exists());
        }
        let next = resolve(&service, OWNER, "recreated");
        assert_ne!(old.id(), next.id());
        assert_ne!(old.identity().generation, next.identity().generation);
        assert_eq!(next.committed().unwrap().version, 0);
        initialize(&next);
        let lease = queue(&next, "a", TransactionMode::ReadOnly);
        start(&next, "a", TransactionMode::ReadOnly, &lease);
        assert!(query(&next).is_empty());
        execute(
            &next,
            Command::Commit {
                transaction: TX,
                quota: None,
            },
        )
        .unwrap();
        assert!(old.submit(Command::DeleteDatabase, 0, |_| {}).is_err());
        verify_empty_namespace(&service, b"other-owner");
        service.shutdown().unwrap();
    }
}

fn verify_empty_namespace(service: &Service, owner: &[u8]) {
    assert!(service.databases(owner).unwrap().is_empty());
    assert_eq!(service.database_version(owner, "recreated").unwrap(), None);
}

#[test]
fn catalog_failed_trash_move_preserves_data_and_blocks_recreation_until_recovery() {
    let directory = Directory::new();
    let root = directory.0.join("idb-lite");
    let mut service = Service::on_disk(root.clone(), config()).unwrap();
    let old = resolve(&service, OWNER, "kept");
    seed(&old);
    let file = path(&old);
    let trash = root.join("trash");
    fs::remove_dir(&trash).unwrap();
    fs::write(&trash, b"blocked").unwrap();
    assert!(execute(&old, Command::DeleteDatabase).is_err());
    assert!(file.exists());
    assert!(
        service
            .resolve(OWNER.to_vec(), "kept".into(), |_| {})
            .is_err()
    );
    assert!(service.databases(OWNER).is_err());
    service.shutdown().unwrap();
    fs::remove_file(&trash).unwrap();
    fs::create_dir(trash).unwrap();
    let recovered = Service::on_disk(root, config()).unwrap();
    verify_record(&resolve(&recovered, OWNER, "kept"));
}

#[test]
fn catalog_uncertain_delete_never_recreates_in_process_and_restart_uses_live_trash_locations() {
    for boundary in [Boundary::Renamed, Boundary::SourceSynced] {
        let directory = Directory::new();
        let root = directory.0.join("idb-lite");
        let mut service = Service::on_disk(root.clone(), config()).unwrap();
        let old = resolve(&service, OWNER, "uncertain");
        seed(&old);
        service.catalog.as_ref().unwrap().interrupt_at(boundary);
        assert!(execute(&old, Command::DeleteDatabase).is_err());
        assert!(!path(&old).parent().unwrap().exists());
        assert!(service.databases(OWNER).is_err());
        assert!(service.database_version(OWNER, "uncertain").is_err());
        assert!(
            service
                .resolve(OWNER.to_vec(), "uncertain".into(), |_| {})
                .is_err()
        );
        assert_eq!(fs::read_dir(root.join("trash")).unwrap().count(), 1);
        service.shutdown().unwrap();
        let recovered = Service::on_disk(root.clone(), config()).unwrap();
        assert!(recovered.databases(OWNER).unwrap().is_empty());
        assert_eq!(fs::read_dir(root.join("trash")).unwrap().count(), 0);
        let next = resolve(&recovered, OWNER, "uncertain");
        assert_ne!(next.identity().generation, old.identity().generation);
    }
}

#[test]
fn catalog_recovery_rejects_duplicates_unknown_formats_and_path_identity_mismatches() {
    let directory = Directory::new();
    let root = directory.0.join("idb-lite");
    let mut service = Service::on_disk(root.clone(), config()).unwrap();
    let first = resolve(&service, OWNER, "first");
    let second = resolve(&service, OWNER, "second");
    seed(&first);
    seed(&second);
    let file = path(&first);
    service.shutdown().unwrap();
    assert!(path(&first).exists() && path(&second).exists());
    let mut restored = Service::on_disk(root.clone(), config()).unwrap();
    assert_eq!(restored.databases(OWNER).unwrap().len(), 2);
    restored.shutdown().unwrap();
    let format = root.join("format.json");
    let bytes = fs::read(&format).unwrap();
    fs::write(&format, b"{\"backend\":\"indexeddb\",\"layout\":2}\n").unwrap();
    assert!(Service::on_disk(root.clone(), config()).is_err());
    fs::write(&format, bytes).unwrap();
    let copied = file
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("07070707070707070707070707070707");
    fs::create_dir(&copied).unwrap();
    fs::copy(&file, copied.join("db.sqlite")).unwrap();
    assert!(Service::on_disk(root.clone(), config()).is_err());
    let connection = moli_sqlite::open_disk(
        &copied.join("db.sqlite"),
        OpenMode::Existing,
        config().connection,
    )
    .unwrap();
    connection
        .execute(
            "UPDATE database_metadata SET generation=?1",
            [&[7_u8; 16][..]],
        )
        .unwrap();
    connection.close().unwrap();
    assert!(Service::on_disk(root, config()).is_err());
    assert!(file.exists() && copied.join("db.sqlite").exists());
}

#[test]
fn catalog_refuses_a_second_owner_and_unsupported_sql_schema_without_erasing_data() {
    let directory = Directory::new();
    let root = directory.0.join("idb-lite");
    let mut service = Service::on_disk(root.clone(), config()).unwrap();
    let client = resolve(&service, OWNER, "protected");
    seed(&client);
    assert!(Service::on_disk(root.clone(), config()).is_err());
    verify_record(&client);
    service.shutdown().unwrap();
    let file = path(&client);
    let connection =
        moli_sqlite::open_disk(&file, OpenMode::Existing, config().connection).unwrap();
    connection.pragma_update(None, "user_version", 2).unwrap();
    connection.close().unwrap();
    assert!(Service::on_disk(root.clone(), config()).is_err());
    let connection =
        moli_sqlite::open_disk(&file, OpenMode::Existing, config().connection).unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
    connection.pragma_update(None, "user_version", 1).unwrap();
    connection.close().unwrap();
    let recovered = Service::on_disk(root, config()).unwrap();
    verify_record(&resolve(&recovered, OWNER, "protected"));
}

#[test]
fn catalog_delete_finishes_even_with_all_data_bytes_retained() {
    let service = Service::new(config()).unwrap();
    let client = resolve(&service, OWNER, "bounded");
    initialize(&client);
    let data = client
        .try_reserve(config().executor.max_in_flight_bytes)
        .unwrap();
    assert_eq!(service.database_version(OWNER, "bounded").unwrap(), Some(1));
    execute(&client, Command::DeleteDatabase).unwrap();
    assert_eq!(service.database_version(OWNER, "bounded").unwrap(), None);
    drop(data);
}

#[test]
fn catalog_does_not_follow_directory_or_database_symlinks() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let directory = Directory::new();
        let root = directory.0.join("idb-lite");
        let mut service = Service::on_disk(root.clone(), config()).unwrap();
        let client = resolve(&service, OWNER, "link");
        initialize(&client);
        let file = path(&client);
        service.shutdown().unwrap();
        let outside = directory.0.join("outside.sqlite");
        fs::rename(&file, &outside).unwrap();
        symlink(&outside, &file).unwrap();
        assert!(Service::on_disk(root, config()).is_err());
        assert!(outside.exists());
    }
}

#[test]
fn catalog_usage_includes_only_committed_live_databases_for_the_exact_owner() {
    let service = Service::new(config()).unwrap();
    let first = resolve(&service, OWNER, "first");
    let second = resolve(&service, OWNER, "second");
    let foreign = resolve(&service, b"different-bucket", "first");
    assert_eq!(service.owner_usage_bytes(OWNER).unwrap(), 0);
    for client in [&first, &second, &foreign] {
        initialize(client);
    }
    let before = service.owner_usage_bytes(OWNER).unwrap();
    assert_eq!(
        before,
        first.committed().unwrap().usage + second.committed().unwrap().usage
    );
    assert_eq!(
        service.owner_usage_bytes(b"different-bucket").unwrap(),
        foreign.committed().unwrap().usage
    );
    let lease = queue(&first, "a", TransactionMode::ReadWrite);
    start(&first, "a", TransactionMode::ReadWrite, &lease);
    write(&first, &[1; 8192]);
    assert_eq!(service.owner_usage_bytes(OWNER).unwrap(), before);
    execute(&first, Command::Abort { transaction: TX }).unwrap();
    assert_eq!(service.owner_usage_bytes(OWNER).unwrap(), before);
    let lease = queue(&first, "a", TransactionMode::ReadWrite);
    start(&first, "a", TransactionMode::ReadWrite, &lease);
    write(&first, &[1; 8192]);
    execute(
        &first,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
    assert!(service.owner_usage_bytes(OWNER).unwrap() > before);
    execute(&first, Command::DeleteDatabase).unwrap();
    assert_eq!(
        service.owner_usage_bytes(OWNER).unwrap(),
        second.committed().unwrap().usage
    );
    assert_eq!(service.owner_usage_bytes(b"missing").unwrap(), 0);
}

#[test]
fn catalog_root_initialization_is_retryable_but_never_fills_holes_in_published_or_unknown_layouts()
{
    let directory = Directory::new();
    let root = directory.0.join("idb-lite");
    fs::create_dir(&root).unwrap();
    fs::create_dir(root.join("live")).unwrap();
    fs::write(
        root.join(".format-00000000000000000000000000000000.tmp"),
        b"partial",
    )
    .unwrap();
    let mut recovered = Service::on_disk(root.clone(), config()).unwrap();
    assert!(recovered.databases(OWNER).unwrap().is_empty());
    recovered.shutdown().unwrap();
    fs::remove_dir(root.join("live")).unwrap();
    assert!(Service::on_disk(root.clone(), config()).is_err());
    assert!(!root.join("live").exists());
    let unknown = directory.0.join("future");
    fs::create_dir(&unknown).unwrap();
    let marker = unknown.join("format.json");
    fs::write(&marker, b"{\"backend\":\"indexeddb\",\"layout\":2}").unwrap();
    assert!(Service::on_disk(unknown.clone(), config()).is_err());
    assert_eq!(fs::read_dir(&unknown).unwrap().count(), 1);
    assert!(!unknown.join("live").exists());
    let old = directory.0.join("not-our-root");
    fs::create_dir(&old).unwrap();
    fs::write(old.join("owner.json"), b"legacy or foreign").unwrap();
    assert!(Service::on_disk(old.clone(), config()).is_err());
    assert!(!old.join("format.json").exists());
    assert_eq!(
        fs::read(old.join("owner.json")).unwrap(),
        b"legacy or foreign"
    );
}
