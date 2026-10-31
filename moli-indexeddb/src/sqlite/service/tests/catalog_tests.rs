use super::super::catalog::Boundary;
use super::*;
use std::fs;

mod crash_tests;
mod idle_tests;
mod lazy_blob_tests;
mod open_retry_tests;

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
                    durability: crate::TransactionDurability::Strict,
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
            .map(|metadata| String::from_utf16(metadata.name.as_utf16()).unwrap())
            .collect::<Vec<_>>(),
        ["\u{10000}", "\u{e000}"]
    );
    assert!(Arc::ptr_eq(&snapshot[0], &astral.committed().unwrap()));
    execute(
        &astral,
        Command::Begin {
            transaction: TX,
            mode: TransactionMode::VersionChange,
            durability: crate::TransactionDurability::Strict,
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
    #[cfg(windows)]
    let root = root.canonicalize().unwrap();
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
fn catalog_closed_disk_owner_reuses_its_known_path() {
    let directory = Directory::new();
    let service = Service::on_disk(directory.0.join("idb-lite"), config()).unwrap();
    let first = resolve(&service, OWNER, "same");
    seed(&first);
    execute(&first, Command::Close).unwrap();

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
        // A failed creation has not published a database. It can be retried
        // in process, or discarded by recovery if the caller leaves it alone.
        assert_eq!(service.database_version(OWNER, "new").unwrap(), None);
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
        old.submit(Command::DeleteDatabase, move |reply| {
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
        assert!(old.submit(Command::DeleteDatabase, |_| {}).is_err());
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
    // This native fixture has no profile lock. Keep a Web connection open so
    // it specifically tests the physical connection's exclusive SQLite lock.
    let _connection = client.open_connection_for_test(Arc::new(|| {})).unwrap();
    seed(&client);
    assert!(Service::on_disk(root.clone(), config()).is_err());
    verify_record(&client);
    service.shutdown().unwrap();
    let file = path(&client);
    let connection =
        moli_sqlite::open_disk(&file, OpenMode::Existing, config().connection).unwrap();
    connection
        .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
        .unwrap();
    connection.close().unwrap();
    assert!(Service::on_disk(root.clone(), config()).is_err());
    let connection =
        moli_sqlite::open_disk(&file, OpenMode::Existing, config().connection).unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        i64::from(SCHEMA_VERSION + 1)
    );
    connection
        .pragma_update(None, "user_version", SCHEMA_VERSION)
        .unwrap();
    connection.close().unwrap();
    let recovered = Service::on_disk(root, config()).unwrap();
    verify_record(&resolve(&recovered, OWNER, "protected"));
}

#[test]
fn catalog_delete_removes_database_version() {
    let service = Service::new(config()).unwrap();
    let client = resolve(&service, OWNER, "bounded");
    initialize(&client);

    assert_eq!(service.database_version(OWNER, "bounded").unwrap(), Some(1));
    execute(&client, Command::DeleteDatabase).unwrap();
    assert_eq!(service.database_version(OWNER, "bounded").unwrap(), None);
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

fn resolve_dom_name(service: &Service, name: crate::IndexedDbName) -> DatabaseClient {
    let (sender, receiver) = mpsc::channel();
    let client = service
        .resolve_named(OWNER.to_vec(), name, move |reply| {
            let _ = sender.send(reply);
        })
        .unwrap();
    receive(&receiver).result.unwrap();
    client
}

#[test]
fn catalog_utf16_database_names_survive_recovery_without_path_or_identity_collisions() {
    let directory = Directory::new();
    let root = directory.0.join("idb-lite");
    let mut names = [
        vec![],
        vec![0],
        vec![0xd800],
        vec![0xd801],
        vec![0xfffd],
        vec![0xd800, 0xdc00],
        vec![0xdc00, 0xd800],
        vec![0xd800, 0],
    ]
    .into_iter()
    .map(crate::IndexedDbName::from_utf16)
    .collect::<Vec<_>>();
    names.sort();
    let mut first_paths = None;
    for write in [true, false] {
        let mut service = Service::on_disk(root.clone(), config()).unwrap();
        let mut paths = Vec::new();
        for name in &names {
            let client = resolve_dom_name(&service, name.clone());
            if write {
                seed(&client);
            }
            verify_record(&client);
            assert_eq!(&client.identity().name, name);
            assert_eq!(
                service.database_version_named(OWNER, name).unwrap(),
                Some(1)
            );
            paths.push(path(&client));
        }
        assert_eq!(
            service
                .databases(OWNER)
                .unwrap()
                .into_iter()
                .map(|metadata| metadata.name.clone())
                .collect::<Vec<_>>(),
            names
        );
        assert_eq!(
            paths
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            names.len()
        );
        if let Some(first) = &first_paths {
            assert_eq!(&paths, first);
        } else {
            first_paths = Some(paths);
        }
        service.shutdown().unwrap();
    }
}

#[test]
fn utf16_connection_requests_share_only_the_exact_code_unit_name() {
    let service = Service::new(config()).unwrap();
    let surrogate = crate::IndexedDbName::from_utf16(vec![0xd800]);
    let replacement = crate::IndexedDbName::from("\u{fffd}");
    let first = service
        .queue_connection_request_named(OWNER, &surrogate, Arc::new(|| {}))
        .unwrap();
    let second = service
        .queue_connection_request_named(OWNER, &surrogate, Arc::new(|| {}))
        .unwrap();
    let independent = service
        .queue_connection_request_named(OWNER, &replacement, Arc::new(|| {}))
        .unwrap();
    assert!(first.handle().is_head());
    assert!(!second.handle().is_head());
    assert!(independent.handle().is_head());
    drop(first);
    assert!(second.handle().is_head());
    assert!(independent.handle().is_head());
}

#[test]
fn utf16_database_names_keep_transaction_admission_separate() {
    let service = Service::new(config()).unwrap();
    let clients = [0xd800, 0xdfff, 0xfffd].map(|unit| {
        let client = resolve_dom_name(&service, crate::IndexedDbName::from_utf16(vec![unit]));
        initialize(&client);
        client
    });
    let leases = clients
        .each_ref()
        .map(|client| queue(client, "a", TransactionMode::ReadWrite));
    let same = queue(&clients[0], "a", TransactionMode::ReadWrite);
    assert!(leases.iter().all(|lease| lease.handle().is_ready()));
    assert!(!same.handle().is_ready());
    for (index, (client, lease)) in clients.iter().zip(leases).enumerate() {
        start(client, "a", TransactionMode::ReadWrite, &lease);
        write(client, &[index as u8]);
        execute(
            client,
            Command::Commit {
                transaction: TX,
                quota: None,
            },
        )
        .unwrap();
    }
    assert!(same.handle().is_ready());
    start(&clients[0], "a", TransactionMode::ReadWrite, &same);
    assert_eq!(query(&clients[0])[0].value, Some(vec![0].into()));
    execute(
        &clients[0],
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
}

#[test]
fn utf16_database_name_quota_rollback_preserves_other_database_versions_and_usage() {
    let service = Service::new(config()).unwrap();
    let clients = [0xd800, 0xdfff, 0xfffd].map(|unit| {
        let client = resolve_dom_name(&service, crate::IndexedDbName::from_utf16(vec![unit]));
        execute(
            &client,
            Command::Begin {
                transaction: TX,
                mode: TransactionMode::VersionChange,
                durability: crate::TransactionDurability::Strict,
                stores: BTreeSet::new(),
                version: Some(1),
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
        client
    });
    assert_eq!(service.owner_usage_bytes(OWNER).unwrap(), 33);
    let client = &clients[0];
    execute(
        client,
        Command::Begin {
            transaction: TX,
            mode: TransactionMode::VersionChange,
            durability: crate::TransactionDurability::Strict,
            stores: BTreeSet::new(),
            version: Some(2),
            admission: None,
        },
    )
    .unwrap();
    execute(
        client,
        Command::CreateStore {
            transaction: TX,
            name: "new".into(),
            options: Default::default(),
        },
    )
    .unwrap();
    assert!(matches!(
        execute(
            client,
            Command::Commit {
                transaction: TX,
                quota: Some(QuotaCheck {
                    quota: 33,
                    other_usage: 22
                })
            }
        ),
        Err(IndexedDbError::QuotaExceeded { .. })
    ));
    assert_eq!(service.owner_usage_bytes(OWNER).unwrap(), 33);
    assert!(
        service
            .databases(OWNER)
            .unwrap()
            .iter()
            .all(|metadata| metadata.version == 1)
    );
}

#[test]
fn catalog_recovers_legacy_text_names_in_the_existing_generation_directory() {
    let directory = Directory::new();
    let root = directory.0.join("idb-lite");
    let legacy_name = "legacy-\0-数据库-\u{1f4be}";
    let mut service = Service::on_disk(root.clone(), config()).unwrap();
    let client = resolve(&service, OWNER, legacy_name);
    seed(&client);
    let file = path(&client);
    let generation = client.identity().generation;
    service.shutdown().unwrap();
    // Recreate the exact v1 metadata representation while keeping this actual
    // generation's records, store ids and allocator counters untouched.
    let mut connection = Connection::open(&file).unwrap();
    let transaction = connection.transaction().unwrap();
    transaction
        .execute_batch("ALTER TABLE database_metadata RENAME TO database_metadata_v2")
        .unwrap();
    let schema = include_str!("../../tests/fixtures/schema-v1.sql");
    let metadata = schema
        .split_once("CREATE TABLE database_metadata")
        .unwrap()
        .1
        .split_once("CREATE TABLE object_stores")
        .unwrap()
        .0;
    transaction
        .execute_batch(&format!("CREATE TABLE database_metadata{metadata}"))
        .unwrap();
    transaction.execute("INSERT INTO database_metadata SELECT singleton,?1,generation,version,state,key_format,key_path_format,value_format,usage,next_store_id,next_index_id,next_record_id,next_object_id FROM database_metadata_v2",[legacy_name]).unwrap();
    transaction
        .execute_batch("DROP TABLE database_metadata_v2; PRAGMA user_version=1")
        .unwrap();
    // Convert the fixture's label tables back too: v1/v2 use TEXT, while
    // the freshly seeded service now uses schema-v3 UTF-16 BLOBs.
    transaction.execute_batch("ALTER TABLE object_stores RENAME TO object_stores_v3; ALTER TABLE indexes RENAME TO indexes_v3").unwrap();
    let labels = schema
        .split_once("CREATE TABLE object_stores")
        .unwrap()
        .1
        .split_once("CREATE TABLE records")
        .unwrap()
        .0;
    transaction
        .execute_batch(&format!("CREATE TABLE object_stores{labels}"))
        .unwrap();
    for (table, id_column, columns) in [
        (
            "object_stores",
            "store_id",
            "store_id,?1,key_path,auto_increment,generator",
        ),
        (
            "indexes",
            "index_id",
            "index_id,store_id,?1,key_path,is_unique,multi_entry",
        ),
    ] {
        let mut statement = transaction
            .prepare(&format!("SELECT {id_column},name FROM {table}_v3"))
            .unwrap();
        let mut rows = statement.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            let id: i64 = row.get(0).unwrap();
            let raw: Vec<u8> = row.get(1).unwrap();
            let name = names::decode(&raw).unwrap();
            let name =
                String::from_utf16(name.as_utf16()).expect("legacy fixture names are Unicode");
            transaction
                .execute(
                    &format!(
                        "INSERT INTO {table} SELECT {columns} FROM {table}_v3 WHERE {id_column}=?2"
                    ),
                    rusqlite::params![name, id],
                )
                .unwrap();
        }
    }
    transaction
        .execute_batch("DROP TABLE indexes_v3; DROP TABLE object_stores_v3")
        .unwrap();
    transaction.commit().unwrap();
    drop(connection);
    let mut service = Service::on_disk(root, config()).unwrap();
    let client = resolve(&service, OWNER, legacy_name);
    assert_eq!(
        client.identity().name,
        crate::IndexedDbName::from(legacy_name)
    );
    assert_eq!(client.identity().generation, generation);
    assert_eq!(path(&client), file);
    verify_record(&client);
    service.shutdown().unwrap();
    let connection = Connection::open(&file).unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .unwrap(),
        SCHEMA_VERSION
    );
    assert_eq!(
        connection
            .query_row("SELECT typeof(name) FROM database_metadata", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap(),
        "blob"
    );
    drop(connection);
}
