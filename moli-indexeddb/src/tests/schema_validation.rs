use super::*;

#[test]
fn schema_validation_preserves_exception_order_and_does_not_create_invalid_metadata() {
    let mut manager = IndexedDbManager::new_in_memory();
    let open = manager
        .open(OpenOptions {
            origin: "origin".into(),
            name: "schema".into(),
            version: Some(1),
        })
        .unwrap();
    let upgrade = open.upgrade_transaction.unwrap();
    manager
        .create_object_store(upgrade, "existing", ObjectStoreOptions::default())
        .unwrap();
    let invalid = ObjectStoreOptions {
        key_path: Some(KeyPath::from("invalid path")),
        auto_increment: true,
    };
    for name in ["existing", "invalid"] {
        assert!(matches!(
            manager.create_object_store(upgrade, name, invalid.clone()),
            Err(IndexedDbError::Syntax(_))
        ));
    }
    let empty = ObjectStoreOptions {
        key_path: Some(KeyPath::from("")),
        auto_increment: true,
    };
    assert!(matches!(
        manager.create_object_store(upgrade, "existing", empty.clone()),
        Err(IndexedDbError::Constraint(_))
    ));
    assert!(matches!(
        manager.create_object_store(upgrade, "invalid", empty),
        Err(IndexedDbError::InvalidAccess(_))
    ));
    assert!(matches!(
        manager.create_object_store(
            upgrade,
            "invalid",
            ObjectStoreOptions {
                key_path: Some(KeyPath::Sequence(vec![])),
                auto_increment: true,
            }
        ),
        Err(IndexedDbError::Syntax(_))
    ));
    let valid = IndexOptions {
        key_path: KeyPath::from("id"),
        unique: false,
        multi_entry: false,
    };
    manager
        .create_index(upgrade, "existing", "index", valid)
        .unwrap();
    let invalid = IndexOptions {
        key_path: KeyPath::Sequence(vec!["invalid path".into()]),
        unique: false,
        multi_entry: true,
    };
    assert!(matches!(
        manager.create_index(upgrade, "existing", "index", invalid.clone()),
        Err(IndexedDbError::Constraint(_))
    ));
    assert!(matches!(
        manager.create_index(upgrade, "existing", "invalid", invalid),
        Err(IndexedDbError::Syntax(_))
    ));
    assert!(matches!(
        manager.create_index(
            upgrade,
            "existing",
            "invalid",
            IndexOptions {
                key_path: KeyPath::Sequence(vec!["id".into()]),
                unique: false,
                multi_entry: true,
            }
        ),
        Err(IndexedDbError::InvalidAccess(_))
    ));
    manager.commit_transaction(upgrade).unwrap();
    assert_eq!(
        manager
            .object_store_info(open.database, "existing")
            .unwrap()
            .index_names,
        ["index"]
    );
    assert!(matches!(
        manager.object_store_info(open.database, "invalid"),
        Err(IndexedDbError::NotFound(_))
    ));
}
