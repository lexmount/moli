use super::*;

fn begin_upgrade(client: &DatabaseClient, version: u64) {
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

fn renamed_records(client: &DatabaseClient) -> Vec<crate::RecordSnapshot> {
    let store_id = store_by_name(client, "renamed\0😀");
    let index_id = index_by_name(client, store_id, "renamed\0索引");
    let lease = queue(client, "renamed\0😀", TransactionMode::ReadOnly);
    start(client, "renamed\0😀", TransactionMode::ReadOnly, &lease);
    let CommandResult::Records(records) = execute(
        client,
        Command::Query {
            transaction: TX,
            store: store_id,
            query: Query {
                source: QuerySource::Index(index_id),
                limit: u32::MAX,
                ..Default::default()
            },
        },
    )
    .unwrap() else {
        panic!("index query must return records")
    };
    execute(
        client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
    records
}

#[test]
fn queued_schema_operations_keep_reserved_ids_without_owner_roundtrips() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "queued-rename", Location::Memory);
    initialize(&client);
    let committed = client.committed().unwrap();
    begin_upgrade(&client, 2);
    // IDs are reserved synchronously. No CREATE receipt or name lookup is
    // needed to queue subsequent work while the SQL owner is held.
    let held = hold_database(&client);
    let store_id = client.allocate_store_id().unwrap();
    let index_id = client.allocate_index_id().unwrap();
    let commands = [
        Command::CreateStore {
            transaction: TX,
            store: store_id,
            name: "new".into(),
            options: ObjectStoreOptions {
                auto_increment: true,
                ..Default::default()
            },
        },
        Command::CreateIndex {
            transaction: TX,
            store: store_id,
            index: index_id,
            name: "tag".into(),
            options: IndexOptions {
                key_path: crate::KeyPath::String("tag".into()),
                unique: true,
                multi_entry: false,
            },
        },
        Command::Write {
            transaction: TX,
            store: store_id,
            key: None,
            value: b"kept".to_vec().into(),
            indexes: vec![IndexKeys {
                index: index_id,
                keys: vec!["alpha".into()],
            }],
            add_only: true,
            quota: None,
        },
        Command::RenameStore {
            transaction: TX,
            store: store_id,
            new_name: "renamed\0😀".into(),
        },
        Command::RenameIndex {
            transaction: TX,
            store: store_id,
            index: index_id,
            new_name: "renamed\0索引".into(),
        },
        Command::Write {
            transaction: TX,
            store: store_id,
            key: None,
            value: b"next".to_vec().into(),
            indexes: vec![IndexKeys {
                index: index_id,
                keys: vec!["beta".into()],
            }],
            add_only: true,
            quota: None,
        },
        Command::Query {
            transaction: TX,
            store: store_id,
            query: Query {
                source: QuerySource::Index(index_id),
                limit: u32::MAX,
                ..Default::default()
            },
        },
    ];
    let (sender, receiver) = mpsc::channel();
    for (position, command) in commands.into_iter().enumerate() {
        let sender = sender.clone();
        client
            .submit(command, move |reply| {
                sender.send((position, reply)).unwrap();
            })
            .unwrap();
    }
    drop(sender);
    assert!(Arc::ptr_eq(&committed, &client.committed().unwrap()));
    drop(held);
    for position in 0..7 {
        let (actual, reply) = receiver.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(actual, position);
        assert_eq!(reply.active_transaction, Some(TX));
        let result = reply.result.unwrap();
        match (position, result) {
            (2, CommandResult::Key(key)) => assert_eq!(key, 1.into()),
            (5, CommandResult::Key(key)) => assert_eq!(key, 2.into()),
            (6, CommandResult::Records(records)) => {
                assert_eq!(records.len(), 2);
                assert_eq!(records[0].primary_key, 1.into());
                assert_eq!(records[0].value.as_ref().unwrap().wire_bytes(), b"kept");
                assert_eq!(records[1].primary_key, 2.into());
                assert_eq!(records[1].value.as_ref().unwrap().wire_bytes(), b"next");
            }
            (0 | 1 | 3 | 4, CommandResult::Unit) => {}
            (_, result) => panic!("unexpected reply at {position}: {result:?}"),
        }
    }
    assert!(Arc::ptr_eq(&committed, &client.committed().unwrap()));
    execute(
        &client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
    assert_eq!(client.committed().unwrap().version, 2);
    assert_eq!(renamed_records(&client).len(), 2);
}

#[test]
fn renamed_schema_and_data_survive_disk_restart_and_a_later_aborted_rename() {
    let directory = Directory::new();
    let root = directory.0.join("catalog");
    let owner = b"partition/default/rename-bucket";
    let mut service = Service::on_disk(root.clone(), config()).unwrap();
    let (sender, receiver) = mpsc::channel();
    let client = service
        .resolve(owner.to_vec(), "schema".into(), move |reply| {
            sender.send(reply).unwrap();
        })
        .unwrap();
    receive(&receiver).result.unwrap();
    initialize(&client);
    let store_id = store_by_name(&client, "a");
    let index_id = client.allocate_index_id().unwrap();
    begin_upgrade(&client, 2);
    execute(
        &client,
        Command::CreateIndex {
            transaction: TX,
            store: store_id,
            index: index_id,
            name: "tag".into(),
            options: IndexOptions {
                key_path: crate::KeyPath::String("tag".into()),
                unique: true,
                multi_entry: false,
            },
        },
    )
    .unwrap();
    execute(
        &client,
        Command::Write {
            transaction: TX,
            store: store_id,
            key: None,
            value: b"kept".to_vec().into(),
            indexes: vec![IndexKeys {
                index: index_id,
                keys: vec!["alpha".into()],
            }],
            add_only: true,
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
    let before = client.committed().unwrap();
    let store_id = before.stores[&crate::IndexedDbName::from("a")].id;
    let index_id = before.stores[&crate::IndexedDbName::from("a")].indexes
        [&crate::IndexedDbName::from("tag")]
        .id;
    begin_upgrade(&client, 3);
    execute(
        &client,
        Command::RenameStore {
            transaction: TX,
            store: store_id,
            new_name: "renamed\0😀".into(),
        },
    )
    .unwrap();
    execute(
        &client,
        Command::RenameIndex {
            transaction: TX,
            store: store_id,
            index: index_id,
            new_name: "renamed\0索引".into(),
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
    let after = client.committed().unwrap();
    assert_eq!(
        after.stores[&crate::IndexedDbName::from("renamed\0😀")].id,
        store_id
    );
    assert_eq!(
        after.stores[&crate::IndexedDbName::from("renamed\0😀")].indexes
            [&crate::IndexedDbName::from("renamed\0索引")]
            .id,
        index_id
    );
    let generation = client.identity().generation;
    service.shutdown().unwrap();
    let service = Service::on_disk(root, config()).unwrap();
    let (sender, receiver) = mpsc::channel();
    let client = service
        .resolve(owner.to_vec(), "schema".into(), move |reply| {
            sender.send(reply).unwrap();
        })
        .unwrap();
    receive(&receiver).result.unwrap();
    assert_eq!(client.identity().generation, generation);
    assert_eq!(client.committed().unwrap(), after);
    begin_upgrade(&client, 4);
    execute(
        &client,
        Command::RenameStore {
            transaction: TX,
            store: store_id,
            new_name: "discard".into(),
        },
    )
    .unwrap();
    execute(
        &client,
        Command::RenameIndex {
            transaction: TX,
            store: store_id,
            index: index_id,
            new_name: "discard".into(),
        },
    )
    .unwrap();
    execute(&client, Command::Abort { transaction: TX }).unwrap();
    assert_eq!(client.committed().unwrap(), after);
    let records = renamed_records(&client);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].key, "alpha".into());
    assert_eq!(records[0].primary_key, 1.into());
    assert_eq!(records[0].value.as_ref().unwrap().wire_bytes(), b"kept");
    let lease = queue(&client, "renamed\0😀", TransactionMode::ReadWrite);
    start(&client, "renamed\0😀", TransactionMode::ReadWrite, &lease);
    let CommandResult::Key(key) = execute(
        &client,
        Command::Write {
            transaction: TX,
            store: store_id,
            key: None,
            value: b"next".to_vec().into(),
            indexes: vec![IndexKeys {
                index: index_id,
                keys: vec![],
            }],
            add_only: true,
            quota: None,
        },
    )
    .unwrap() else {
        panic!("generated key")
    };
    assert_eq!(key, 2.into());
    execute(
        &client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
}
