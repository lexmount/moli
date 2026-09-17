use super::*;

fn options(path: &str) -> IndexOptions {
    IndexOptions {
        key_path: KeyPath::from(path),
        unique: true,
        multi_entry: false,
    }
}

#[test]
fn index_rename_is_atomic_and_rolls_back_with_the_upgrade() {
    let mut manager = IndexedDbManager::new_in_memory();
    let open = manager
        .open(OpenOptions {
            origin: "origin".into(),
            name: "rename".into(),
            version: Some(1),
        })
        .unwrap();
    let tx = open.upgrade_transaction.unwrap();
    manager
        .create_object_store(tx, "s", ObjectStoreOptions::default())
        .unwrap();
    manager.create_index(tx, "s", "a", options("key")).unwrap();
    manager
        .create_index(tx, "s", "b", options("other"))
        .unwrap();
    manager.put(tx, "s", Key::number(1.0), vec![7]).unwrap();
    let revision = manager.transaction_record_revision(tx).unwrap();
    let a = IndexedDbName::from("a");
    let b = IndexedDbName::from("b");
    assert!(matches!(
        manager.rename_index(tx, "s", &a, b.clone()),
        Err(IndexedDbError::Constraint(_))
    ));
    manager.rename_index(tx, "s", &a, a.clone()).unwrap();
    manager
        .rename_index(tx, "s", &a, "temporary".into())
        .unwrap();
    manager.rename_index(tx, "s", &b, a.clone()).unwrap();
    manager
        .rename_index(tx, "s", &"temporary".into(), b.clone())
        .unwrap();
    assert_eq!(manager.transaction_record_revision(tx).unwrap(), revision);
    manager.commit_transaction(tx).unwrap();
    assert_eq!(
        manager.index_info(open.database, "s", &a).unwrap().key_path,
        KeyPath::from("other")
    );
    assert_eq!(
        manager.index_info(open.database, "s", &b).unwrap().key_path,
        KeyPath::from("key")
    );
    manager.close_database(open.database).unwrap();
    let upgrade = manager
        .open(OpenOptions {
            origin: "origin".into(),
            name: "rename".into(),
            version: Some(2),
        })
        .unwrap();
    let tx = upgrade.upgrade_transaction.unwrap();
    manager.rename_index(tx, "s", &b, "changed".into()).unwrap();
    manager.delete_index(tx, "s", &a).unwrap();
    manager
        .create_index(tx, "s", "a", options("replacement"))
        .unwrap();
    manager.abort_transaction(tx).unwrap();
    assert_eq!(
        manager
            .index_info(upgrade.database, "s", &a)
            .unwrap()
            .key_path,
        KeyPath::from("other")
    );
    assert_eq!(
        manager
            .index_info(upgrade.database, "s", &b)
            .unwrap()
            .key_path,
        KeyPath::from("key")
    );
    assert!(matches!(
        manager.index_info(upgrade.database, "s", "changed"),
        Err(IndexedDbError::NotFound(_))
    ));
    let tx = manager
        .begin_transaction(upgrade.database, &["s".into()], TransactionMode::ReadWrite)
        .unwrap();
    assert!(matches!(
        manager.rename_index(tx, "s", &a, "changed".into()),
        Err(IndexedDbError::InvalidState(_))
    ));
    assert_eq!(
        manager.get(tx, "s", &Key::number(1.0).unwrap()).unwrap(),
        RequestOutcome::Value(Some(vec![7].into()))
    );
}

#[test]
fn index_names_preserve_utf16_in_metadata_and_persistence_while_reading_legacy_files() {
    let dir = TestDir::new();
    let origin = "https://example.test";
    let mut manager = IndexedDbManager::new(&dir.path).unwrap();
    let open = manager
        .open(OpenOptions {
            origin: origin.into(),
            name: "names".into(),
            version: Some(1),
        })
        .unwrap();
    let tx = open.upgrade_transaction.unwrap();
    manager
        .create_object_store(tx, "s", ObjectStoreOptions::default())
        .unwrap();
    manager
        .create_index(tx, "s", "legacy", options("key"))
        .unwrap();
    manager.commit_transaction(tx).unwrap();
    manager.close_database(open.database).unwrap();
    drop(manager);
    // A well-formed name still uses the original JSON object representation.
    let legacy: serde_json::Value =
        serde_json::from_slice(&fs::read(origin_path(&dir.path, origin)).unwrap()).unwrap();
    let store = &legacy["databases"]["names"]["stores"]["s"];
    assert_eq!(store["indexes"]["legacy"]["unique"], true);
    assert!(store.get("indexes_utf16").is_none());

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
    let open = manager
        .open(OpenOptions {
            origin: origin.into(),
            name: "names".into(),
            version: Some(2),
        })
        .unwrap();
    let tx = open.upgrade_transaction.unwrap();
    manager
        .rename_index(tx, "s", &"legacy".into(), names[2].clone())
        .unwrap();
    for (number, name) in names.iter().enumerate() {
        if number != 2 {
            manager
                .create_index(tx, "s", name, options(&format!("key{number}")))
                .unwrap();
        }
    }
    manager.commit_transaction(tx).unwrap();
    let usage = manager.origin_usage_bytes(origin).unwrap();
    manager.close_database(open.database).unwrap();
    drop(manager);
    let mut manager = IndexedDbManager::new(&dir.path).unwrap();
    let open = manager
        .open(OpenOptions {
            origin: origin.into(),
            name: "names".into(),
            version: None,
        })
        .unwrap();
    let mut ordered = names.clone();
    ordered.sort();
    assert_eq!(
        manager
            .object_store_info(open.database, "s")
            .unwrap()
            .index_names,
        ordered
    );
    for (number, name) in names.iter().enumerate() {
        let info = manager.index_info(open.database, "s", name).unwrap();
        assert_eq!(&info.name, name);
        assert_eq!(
            info.key_path,
            KeyPath::from(if number == 2 {
                "key".into()
            } else {
                format!("key{number}")
            })
        );
    }
    assert_eq!(manager.origin_usage_bytes(origin).unwrap(), usage);
}
