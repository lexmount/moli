use super::*;

fn upgrade(client: &DatabaseClient, version: u64) {
    execute(
        client,
        Command::Begin {
            transaction: TX,
            mode: TransactionMode::VersionChange,
            stores: BTreeSet::new(),
            version: Some(version),
            admission: None,
        },
    )
    .unwrap();
}

fn create(client: &DatabaseClient, store: StoreId, index: IndexId) {
    execute(
        client,
        Command::CreateStore {
            transaction: TX,
            store,
            name: "new".into(),
            options: ObjectStoreOptions::default(),
        },
    )
    .unwrap();
    execute(
        client,
        Command::CreateIndex {
            transaction: TX,
            store,
            index,
            name: "tag".into(),
            options: IndexOptions {
                key_path: crate::KeyPath::String("tag".into()),
                unique: false,
                multi_entry: false,
            },
        },
    )
    .unwrap();
}

#[test]
fn schema_id_reservations_are_shared_across_clients_and_reject_retired_databases() {
    let mut service = Service::new(config()).unwrap();
    let client = open(&service, "shared-schema-ids", Location::Memory);
    let ids = std::thread::scope(|scope| {
        let workers = (0..4)
            .map(|_| {
                let client = client.clone();
                scope.spawn(move || {
                    (0..100)
                        .map(|_| {
                            (
                                client.allocate_store_id().unwrap(),
                                client.allocate_index_id().unwrap(),
                            )
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    let stores = ids.iter().map(|&(store, _)| store).collect::<BTreeSet<_>>();
    let indexes = ids.iter().map(|&(_, index)| index).collect::<BTreeSet<_>>();
    assert_eq!(stores.len(), 400);
    assert_eq!(indexes.len(), 400);
    service.shutdown().unwrap();
    assert!(client.allocate_store_id().is_err());
    assert!(client.allocate_index_id().is_err());
}

#[test]
fn schema_ids_survive_abort_idle_reconnect_and_disk_restart() {
    let directory = Directory::new();
    let path = directory.0.join("schema-ids.sqlite");
    let mut service = Service::new(config()).unwrap();
    let client = open(
        &service,
        "schema-ids",
        Location::Disk {
            path: path.clone(),
            mode: OpenMode::Create,
        },
    );
    initialize(&client);
    upgrade(&client, 2);
    let removed_store = client.allocate_store_id().unwrap();
    let removed_index = client.allocate_index_id().unwrap();
    create(&client, removed_store, removed_index);
    execute(&client, Command::Abort { transaction: TX }).unwrap();
    let reserved_store = client.allocate_store_id().unwrap();
    let reserved_index = client.allocate_index_id().unwrap();
    assert!(reserved_store > removed_store);
    assert!(reserved_index > removed_index);
    reap_idle(&service);
    assert!(!physical_connection(&client));
    // The physical connection reopens before the next allocation. An older
    // persisted high-water mark cannot rewind reservations held by clients.
    upgrade(&client, 2);
    let store = client.allocate_store_id().unwrap();
    let index = client.allocate_index_id().unwrap();
    assert!(store > reserved_store);
    assert!(index > reserved_index);
    create(&client, store, index);
    execute(
        &client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
    assert_eq!(
        client.committed().unwrap().stores[&crate::IndexedDbName::from("new")].id,
        store
    );
    service.shutdown().unwrap();

    let service = Service::new(config()).unwrap();
    let reopened = open(
        &service,
        "schema-ids",
        Location::Disk {
            path,
            mode: OpenMode::Existing,
        },
    );
    let metadata = reopened.committed().unwrap();
    let restored = &metadata.stores[&crate::IndexedDbName::from("new")];
    assert_eq!(restored.id, store);
    assert_eq!(
        restored.indexes[&crate::IndexedDbName::from("tag")].id,
        index
    );
    assert!(reopened.allocate_store_id().unwrap() > store);
    assert!(reopened.allocate_index_id().unwrap() > index);
}

#[test]
fn retained_ids_do_not_follow_reused_store_or_index_names() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "id-targets", Location::Memory);
    initialize(&client);
    let store = store_by_name(&client, "a");
    let index = client.allocate_index_id().unwrap();
    upgrade(&client, 2);
    execute(
        &client,
        Command::CreateIndex {
            transaction: TX,
            store,
            index,
            name: "tag".into(),
            options: IndexOptions {
                key_path: crate::KeyPath::String("tag".into()),
                unique: false,
                multi_entry: false,
            },
        },
    )
    .unwrap();
    execute(
        &client,
        Command::Write {
            transaction: TX,
            store,
            key: Some(1.into()),
            value: b"original".to_vec().into(),
            indexes: vec![IndexKeys {
                index,
                keys: vec!["key".into()],
            }],
            add_only: false,
            quota: None,
        },
    )
    .unwrap();
    execute(
        &client,
        Command::RenameStore {
            transaction: TX,
            store,
            new_name: "kept".into(),
        },
    )
    .unwrap();
    execute(
        &client,
        Command::RenameIndex {
            transaction: TX,
            store,
            index,
            new_name: "kept-index".into(),
        },
    )
    .unwrap();
    let replacement_store = client.allocate_store_id().unwrap();
    execute(
        &client,
        Command::CreateStore {
            transaction: TX,
            store: replacement_store,
            name: "a".into(),
            options: ObjectStoreOptions::default(),
        },
    )
    .unwrap();
    execute(
        &client,
        Command::Write {
            transaction: TX,
            store: replacement_store,
            key: Some(1.into()),
            value: b"replacement".to_vec().into(),
            indexes: vec![],
            add_only: false,
            quota: None,
        },
    )
    .unwrap();
    let replacement_index = client.allocate_index_id().unwrap();
    execute(
        &client,
        Command::CreateIndex {
            transaction: TX,
            store,
            index: replacement_index,
            name: "tag".into(),
            options: IndexOptions {
                key_path: crate::KeyPath::String("other".into()),
                unique: false,
                multi_entry: false,
            },
        },
    )
    .unwrap();
    let CommandResult::Records(rows) = execute(
        &client,
        Command::Query {
            transaction: TX,
            store,
            query: Query {
                source: QuerySource::Index(index),
                ..Default::default()
            },
        },
    )
    .unwrap() else {
        panic!("expected original index rows")
    };
    assert_eq!(rows[0].value.as_ref().unwrap().wire_bytes(), b"original");
    // An index ID cannot be applied to a different object store.
    assert!(matches!(
        execute(
            &client,
            Command::Count {
                transaction: TX,
                store: replacement_store,
                source: QuerySource::Index(index),
                range: Default::default(),
            }
        ),
        Err(IndexedDbError::NotFound(_))
    ));
    execute(
        &client,
        Command::Delete {
            transaction: TX,
            store,
            range: Default::default(),
        },
    )
    .unwrap();
    let CommandResult::Records(rows) = execute(
        &client,
        Command::Query {
            transaction: TX,
            store: replacement_store,
            query: Query::default(),
        },
    )
    .unwrap() else {
        panic!("expected replacement rows")
    };
    assert_eq!(rows[0].value.as_ref().unwrap().wire_bytes(), b"replacement");
    execute(
        &client,
        Command::DeleteIndex {
            transaction: TX,
            store,
            index,
        },
    )
    .unwrap();
    assert!(matches!(
        execute(
            &client,
            Command::Query {
                transaction: TX,
                store,
                query: Query {
                    source: QuerySource::Index(index),
                    ..Default::default()
                },
            }
        ),
        Err(IndexedDbError::NotFound(_))
    ));
    execute(
        &client,
        Command::DeleteStore {
            transaction: TX,
            store,
        },
    )
    .unwrap();
    let same_name = client.allocate_store_id().unwrap();
    execute(
        &client,
        Command::CreateStore {
            transaction: TX,
            store: same_name,
            name: "kept".into(),
            options: ObjectStoreOptions::default(),
        },
    )
    .unwrap();
    assert!(matches!(
        execute(
            &client,
            Command::Query {
                transaction: TX,
                store,
                query: Query::default(),
            }
        ),
        Err(IndexedDbError::NotFound(_))
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
fn reopening_rejects_schema_counters_that_would_reissue_live_ids() {
    for column in ["next_store_id", "next_index_id"] {
        let directory = Directory::new();
        let path = directory.0.join("schema-counter.sqlite");
        let mut service = Service::new(config()).unwrap();
        let client = open(
            &service,
            "corrupt-counter",
            Location::Disk {
                path: path.clone(),
                mode: OpenMode::Create,
            },
        );
        initialize(&client);
        upgrade(&client, 2);
        create(
            &client,
            client.allocate_store_id().unwrap(),
            client.allocate_index_id().unwrap(),
        );
        execute(
            &client,
            Command::Commit {
                transaction: TX,
                quota: None,
            },
        )
        .unwrap();
        with_database(&client, move |database| {
            database
                .connection
                .execute(&format!("UPDATE database_metadata SET {column}=1"), [])
                .unwrap();
        });
        service.shutdown().unwrap();
        let service = Service::new(config()).unwrap();
        let (_, receipt) = register(
            &service,
            "corrupt-counter",
            Location::Disk {
                path,
                mode: OpenMode::Existing,
            },
        );
        assert!(matches!(receipt.result, Err(IndexedDbError::Corruption(_))));
    }
}

#[test]
fn recovered_catalog_reserves_schema_ids_before_reopening_sql() {
    let resolve = |service: &Service| {
        let (sender, receiver) = mpsc::channel();
        let client = service
            .resolve(
                b"schema-id-owner".to_vec(),
                "schema-ids".into(),
                move |reply| {
                    let _ = sender.send(reply);
                },
            )
            .unwrap();
        receive(&receiver).result.unwrap();
        client
    };
    let directory = Directory::new();
    let root = directory.0.join("idb-lite");
    let mut service = Service::on_disk(root.clone(), config()).unwrap();
    let client = resolve(&service);
    initialize(&client);
    upgrade(&client, 2);
    let deleted_store = client.allocate_store_id().unwrap();
    let deleted_index = client.allocate_index_id().unwrap();
    create(&client, deleted_store, deleted_index);
    execute(
        &client,
        Command::DeleteStore {
            transaction: TX,
            store: deleted_store,
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
    service.shutdown().unwrap();

    let service = Service::on_disk(root, config()).unwrap();
    let client = resolve(&service);
    assert!(!physical_connection(&client));
    let store = client.allocate_store_id().unwrap();
    let index = client.allocate_index_id().unwrap();
    assert!(store > deleted_store);
    assert!(index > deleted_index);
    assert!(!physical_connection(&client));
    upgrade(&client, 3);
    create(&client, store, index);
    execute(
        &client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
    let metadata = client.committed().unwrap();
    let created = &metadata.stores[&crate::IndexedDbName::from("new")];
    assert_eq!(created.id, store);
    assert_eq!(
        created.indexes[&crate::IndexedDbName::from("tag")].id,
        index
    );
}
