use super::*;

#[test]
fn metadata_snapshots_share_definitions_and_release_old_versions() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "shared-schema", Location::Memory);
    initialize(&client);
    let before = client.committed().unwrap();
    let old = Arc::downgrade(&before);
    let lease = queue(&client, "a", TransactionMode::ReadWrite);
    start(&client, "a", TransactionMode::ReadWrite, &lease);
    write(&client, b"new contents");
    let CommandResult::Metadata(after) = execute(
        &client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap() else {
        panic!("commit publishes its snapshot");
    };
    assert!(!Arc::ptr_eq(&before, &after));
    assert!(Arc::ptr_eq(&before.stores, &after.stores));
    assert!(after.usage > before.usage);
    let observer = after.clone();
    with_database(&client, move |database| {
        assert!(Arc::ptr_eq(database.committed(), &observer))
    });
    drop(before);
    assert!(old.upgrade().is_none());
    let lease = queue(&client, "a", TransactionMode::ReadOnly);
    start(&client, "a", TransactionMode::ReadOnly, &lease);
    let CommandResult::Metadata(unchanged) = execute(
        &client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap() else {
        panic!("readonly commit publishes the existing snapshot");
    };
    assert!(Arc::ptr_eq(&after, &unchanged));
}

#[test]
fn failed_schema_read_before_commit_rolls_back_and_allows_a_later_upgrade() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "metadata-commit", Location::Memory);
    initialize(&client);
    let original = client.committed().unwrap();
    for fail in [true, false] {
        execute(
            &client,
            Command::Begin {
                transaction: TX,
                mode: TransactionMode::VersionChange,
                stores: BTreeSet::new(),
                version: Some(2),
                admission: None,
            },
        )
        .unwrap();
        execute(
            &client,
            Command::CreateStore {
                transaction: TX,
                name: "new".into(),
                options: Default::default(),
            },
        )
        .unwrap();
        if fail {
            with_database(&client, |database| {
                database
                    .connection
                    .execute_batch("DROP TABLE indexes")
                    .unwrap();
            });
        }
        let result = reply(
            &client,
            Command::Commit {
                transaction: TX,
                quota: None,
            },
        );
        assert_eq!(result.active_transaction, None);
        if fail {
            assert!(matches!(result.result, Err(IndexedDbError::Io(_))));
            assert!(Arc::ptr_eq(&original, &client.committed().unwrap()));
            assert!(
                !client
                    .committed()
                    .unwrap()
                    .stores
                    .contains_key(&crate::IndexedDbName::from("new"))
            );
        } else {
            result.result.unwrap();
            let committed = client.committed().unwrap();
            assert_eq!(committed.version, 2);
            assert!(
                committed
                    .stores
                    .contains_key(&crate::IndexedDbName::from("new"))
            );
        }
    }
}

#[test]
fn shared_snapshots_outlive_shutdown_and_release_at_the_last_owner() {
    let settings = config();
    let service = Service::new(settings).unwrap();
    let clients = ["first", "second"].map(|name| {
        let client = open(&service, name, Location::Memory);
        initialize(&client);
        client
    });
    let snapshot = clients[0].committed().unwrap();
    let definitions = snapshot.stores.clone();
    let weak_snapshot = Arc::downgrade(&snapshot);
    let weak_definitions = Arc::downgrade(&definitions);
    drop(service);
    drop(clients);
    assert!(weak_snapshot.upgrade().is_some());
    drop(snapshot);
    assert!(weak_snapshot.upgrade().is_none());
    assert!(weak_definitions.upgrade().is_some());
    drop(definitions);
    assert!(weak_definitions.upgrade().is_none());
}

#[test]
fn retaining_old_snapshots_does_not_reject_new_commits() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "retained", Location::Memory);
    initialize(&client);
    let mut retained = Vec::new();
    for _ in 0..1000 {
        retained.push(client.committed().unwrap());
        let lease = queue(&client, "a", TransactionMode::ReadWrite);
        start(&client, "a", TransactionMode::ReadWrite, &lease);
        write(&client, b"record");
        execute(
            &client,
            Command::Commit {
                transaction: TX,
                quota: None,
            },
        )
        .unwrap();
    }
    assert!(
        retained
            .iter()
            .all(|snapshot| { Arc::ptr_eq(&snapshot.stores, &retained[0].stores) })
    );
    let first = Arc::downgrade(&retained[0]);
    drop(retained);
    assert!(first.upgrade().is_none());
    let reader = queue(&client, "a", TransactionMode::ReadOnly);
    start(&client, "a", TransactionMode::ReadOnly, &reader);
    assert!(matches!(
        execute(
            &client,
            Command::Count {
                transaction: TX,
                store: "a".into(),
                source: Default::default(),
                range: Default::default(),
            }
        ),
        Ok(CommandResult::Count(1000))
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
