use super::*;

fn open(manager: &mut IndexedDbManager, version: u64) -> OpenResult {
    manager
        .open(OpenOptions {
            origin: "https://rename.test".into(),
            name: "stores".into(),
            version: Some(version),
        })
        .unwrap()
}

#[test]
fn store_rename_preserves_records_indexes_generator_and_rolls_back_atomically() {
    let mut manager = IndexedDbManager::new_in_memory();
    let database = open(&mut manager, 1);
    let tx = database.upgrade_transaction.unwrap();
    manager
        .create_object_store(
            tx,
            "a",
            ObjectStoreOptions {
                auto_increment: true,
                ..Default::default()
            },
        )
        .unwrap();
    manager
        .create_object_store(tx, "b", ObjectStoreOptions::default())
        .unwrap();
    manager
        .create_index(
            tx,
            "a",
            "index",
            IndexOptions {
                key_path: KeyPath::from("field"),
                unique: true,
                multi_entry: false,
            },
        )
        .unwrap();
    assert_eq!(manager.add(tx, "a", None, vec![7]).unwrap(), Key::from(1));
    let revision = manager.transaction_record_revision(tx).unwrap();
    assert!(matches!(
        manager.rename_object_store(tx, &"a".into(), "b".into()),
        Err(IndexedDbError::Constraint(_))
    ));
    assert!(matches!(
        manager.rename_object_store(tx, &"missing".into(), "missing".into()),
        Err(IndexedDbError::NotFound(_))
    ));
    manager
        .rename_object_store(tx, &"a".into(), "a".into())
        .unwrap();
    manager
        .rename_object_store(tx, &"a".into(), "temp".into())
        .unwrap();
    manager
        .rename_object_store(tx, &"b".into(), "a".into())
        .unwrap();
    manager
        .rename_object_store(tx, &"temp".into(), "b".into())
        .unwrap();
    assert_eq!(manager.transaction_record_revision(tx).unwrap(), revision);
    assert_eq!(
        manager.get(tx, "b", &Key::from(1)).unwrap(),
        RequestOutcome::Value(Some(vec![7].into()))
    );
    assert_eq!(manager.add(tx, "b", None, vec![8]).unwrap(), Key::from(2));
    manager.commit_transaction(tx).unwrap();
    assert_eq!(
        manager
            .index_info(database.database, "b", "index")
            .unwrap()
            .key_path,
        KeyPath::from("field")
    );
    manager.close_database(database.database).unwrap();

    let database = open(&mut manager, 2);
    let tx = database.upgrade_transaction.unwrap();
    manager
        .rename_object_store(tx, &"b".into(), "renamed".into())
        .unwrap();
    manager.delete_object_store(tx, "renamed").unwrap();
    manager
        .create_object_store(tx, "b", ObjectStoreOptions::default())
        .unwrap();
    manager.abort_transaction(tx).unwrap();
    assert!(
        manager
            .object_store_info(database.database, "b")
            .unwrap()
            .auto_increment
    );
    assert_eq!(
        manager
            .database_info(database.database)
            .unwrap()
            .object_store_names,
        vec![IndexedDbName::from("a"), IndexedDbName::from("b")]
    );
    let tx = manager
        .begin_transaction(database.database, &["b".into()], TransactionMode::ReadWrite)
        .unwrap();
    assert_eq!(manager.add(tx, "b", None, vec![9]).unwrap(), Key::from(3));
    assert!(matches!(
        manager.rename_object_store(tx, &"b".into(), "x".into()),
        Err(IndexedDbError::InvalidState(_))
    ));
    assert!(manager.get(tx, "a", &Key::from(1)).is_err());
    manager.commit_transaction(tx).unwrap();
}

#[test]
fn store_names_roundtrip_utf16_and_legacy_persistence_without_collisions() {
    let dir = TestDir::new();
    let mut manager = IndexedDbManager::new(&dir.path).unwrap();
    let database = open(&mut manager, 1);
    let tx = database.upgrade_transaction.unwrap();
    manager
        .create_object_store(tx, "legacy", ObjectStoreOptions::default())
        .unwrap();
    manager
        .put(tx, "legacy", Some(Key::from(1)), vec![2])
        .unwrap();
    manager.commit_transaction(tx).unwrap();
    manager.close_database(database.database).unwrap();
    drop(manager);
    let bytes = fs::read(origin_path(&dir.path, "https://rename.test")).unwrap();
    let legacy: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(legacy["databases"]["stores"].get("stores_utf16").is_none());
    assert!(
        legacy["databases"]["stores"]["stores"]
            .get("legacy")
            .is_some()
    );

    let names = [
        vec![],
        vec![0],
        vec![0xd800],
        vec![0xd801],
        vec![0xdc00],
        vec![0xfffd],
        vec![0xdc00, 0xd800],
        vec![0xd83d, 0xde00],
        vec![0xe000],
    ]
    .into_iter()
    .map(IndexedDbName::from_utf16)
    .collect::<Vec<_>>();
    let mut manager = IndexedDbManager::new(&dir.path).unwrap();
    let database = open(&mut manager, 2);
    let tx = database.upgrade_transaction.unwrap();
    manager
        .rename_object_store(tx, &"legacy".into(), names[2].clone())
        .unwrap();
    for (number, name) in names.iter().enumerate() {
        if number != 2 {
            manager
                .create_object_store(tx, name, ObjectStoreOptions::default())
                .unwrap();
            manager
                .put(tx, name, Some(Key::from(1)), vec![number as u8])
                .unwrap();
        }
    }
    manager.commit_transaction(tx).unwrap();
    let usage = manager.origin_usage_bytes("https://rename.test").unwrap();
    manager.close_database(database.database).unwrap();
    drop(manager);
    let mut manager = IndexedDbManager::new(&dir.path).unwrap();
    let database = open(&mut manager, 2);
    let mut expected = names.clone();
    expected.sort();
    assert_eq!(
        manager
            .database_info(database.database)
            .unwrap()
            .object_store_names,
        expected
    );
    for (number, name) in names.iter().enumerate() {
        assert_eq!(
            &manager
                .object_store_info(database.database, name)
                .unwrap()
                .name,
            name
        );
        let tx = manager
            .begin_transaction(
                database.database,
                std::slice::from_ref(name),
                TransactionMode::ReadOnly,
            )
            .unwrap();
        assert_eq!(
            manager.get(tx, name, &Key::from(1)).unwrap(),
            RequestOutcome::Value(Some(vec![number as u8].into()))
        );
        manager.commit_transaction(tx).unwrap();
    }
    assert_eq!(
        manager.origin_usage_bytes("https://rename.test").unwrap(),
        usage
    );
    // A writer for a lone surrogate must not block a different lone surrogate
    // or U+FFFD, but another request for that exact UTF-16 name must wait.
    let wake: crate::ConnectionRequestWake = std::sync::Arc::new(|| {});
    let first = manager
        .queue_transaction_start(
            database.database,
            &names[2..3],
            TransactionMode::ReadWrite,
            wake.clone(),
        )
        .unwrap();
    let other = manager
        .queue_transaction_start(
            database.database,
            &names[3..4],
            TransactionMode::ReadWrite,
            wake.clone(),
        )
        .unwrap();
    let replacement = manager
        .queue_transaction_start(
            database.database,
            &names[5..6],
            TransactionMode::ReadWrite,
            wake.clone(),
        )
        .unwrap();
    let same = manager
        .queue_transaction_start(
            database.database,
            &names[2..3],
            TransactionMode::ReadWrite,
            wake,
        )
        .unwrap();
    assert!(
        first.handle().is_ready() && other.handle().is_ready() && replacement.handle().is_ready()
    );
    assert!(!same.handle().is_ready());
    let first_tx = manager.start_queued_transaction(first.handle()).unwrap();
    let other_tx = manager.start_queued_transaction(other.handle()).unwrap();
    manager
        .put(first_tx, &names[2], Some(Key::from(2)), vec![20])
        .unwrap();
    manager
        .put(other_tx, &names[3], Some(Key::from(2)), vec![30])
        .unwrap();
    manager.commit_transaction(other_tx).unwrap();
    manager.commit_transaction(first_tx).unwrap();
    drop(first);
    assert!(same.handle().is_ready());
    let tx = manager.start_queued_transaction(same.handle()).unwrap();
    assert_eq!(
        manager.get(tx, &names[2], &Key::from(2)).unwrap(),
        RequestOutcome::Value(Some(vec![20].into()))
    );
    manager.commit_transaction(tx).unwrap();
}
