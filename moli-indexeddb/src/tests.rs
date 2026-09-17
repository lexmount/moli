use std::{
    cmp::Ordering as CmpOrdering,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    persistence::{origin_file_stem, origin_path},
    transaction::{MAX_AUTO_INCREMENT_KEY, transaction_store_mut},
    *,
};

struct TestDir {
    path: PathBuf,
}

mod durability;
mod index_rename;
mod record_revision;
mod store_rename;
mod transaction_scheduling;

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
        [IndexedDbName::from("index")]
    );
    assert!(matches!(
        manager.object_store_info(open.database, "invalid"),
        Err(IndexedDbError::NotFound(_))
    ));
}

#[test]
fn closing_connection_defers_release_until_commit_or_forced_abort() {
    for force in [false, true] {
        let mut manager = IndexedDbManager::new_in_memory();
        let open = manager
            .open(OpenOptions {
                origin: "origin".into(),
                name: "close".into(),
                version: Some(1),
            })
            .unwrap();
        let upgrade = open.upgrade_transaction.unwrap();
        manager
            .create_object_store(upgrade, "records", ObjectStoreOptions::default())
            .unwrap();
        manager.commit_transaction(upgrade).unwrap();
        let coordinator = manager.connection_notifications();
        coordinator.register(open.database, "key".into(), std::sync::Arc::new(|| {}));
        let transaction = manager
            .begin_transaction(
                open.database,
                &["records".into()],
                TransactionMode::ReadWrite,
            )
            .unwrap();
        manager.close_database(open.database).unwrap();
        assert!(coordinator.has_connections("key"));
        assert!(
            manager
                .begin_transaction(
                    open.database,
                    &["records".into()],
                    TransactionMode::ReadOnly
                )
                .is_err()
        );
        assert!(manager.delete_database("origin", "close").is_err());
        if force {
            manager.force_close_database(open.database).unwrap();
            assert!(manager.commit_transaction(transaction).is_err());
        } else {
            manager.commit_transaction(transaction).unwrap();
        }
        assert!(!coordinator.has_connections("key"));
        manager.delete_database("origin", "close").unwrap();
    }
}

#[test]
fn cursor_direction_labels_and_flags_follow_spec_tokens() {
    let cases = [
        ("next", CursorDirection::Next, false, false),
        ("nextunique", CursorDirection::NextUnique, false, true),
        ("prev", CursorDirection::Prev, true, false),
        ("prevunique", CursorDirection::PrevUnique, true, true),
    ];

    for (label, direction, reverse, unique) in cases {
        assert_eq!(CursorDirection::parse(label), Some(direction));
        assert_eq!(direction.as_str(), label);
        assert_eq!(direction.is_reverse(), reverse);
        assert_eq!(direction.is_unique(), unique);
    }
    assert_eq!(CursorDirection::parse("forward"), None);
    assert_eq!(CursorDirection::default_next(), CursorDirection::Next);
}

#[test]
fn cursor_direction_helpers_reverse_and_deduplicate_by_key() {
    let entries = vec![
        (Key::from("a"), 1),
        (Key::from("a"), 2),
        (Key::from("b"), 3),
    ];

    assert_eq!(
        apply_collection_direction(entries.clone(), CursorDirection::Prev),
        vec![
            (Key::from("b"), 3),
            (Key::from("a"), 2),
            (Key::from("a"), 1),
        ]
    );

    assert_eq!(
        apply_cursor_direction_by_key(entries.clone(), CursorDirection::NextUnique, |entry| {
            &entry.0
        }),
        vec![(Key::from("a"), 1), (Key::from("b"), 3)]
    );
    assert_eq!(
        apply_cursor_direction_by_key(entries, CursorDirection::PrevUnique, |entry| &entry.0),
        vec![(Key::from("b"), 3), (Key::from("a"), 1)]
    );
}

#[test]
fn cursor_direction_comparisons_respect_reverse_order() {
    assert_eq!(
        compare_cursor_direction(CursorDirection::Next, &Key::from(2), &Key::from(1)),
        CmpOrdering::Greater
    );
    assert_eq!(
        compare_cursor_direction(CursorDirection::Prev, &Key::from(2), &Key::from(1)),
        CmpOrdering::Less
    );
    assert_eq!(
        compare_cursor_tuple_direction(
            CursorDirection::Next,
            &Key::from(2),
            &Key::from(1),
            &Key::from(2),
            &Key::from(0),
        ),
        CmpOrdering::Greater
    );
    assert_eq!(
        compare_cursor_tuple_direction(
            CursorDirection::Prev,
            &Key::from(2),
            &Key::from(1),
            &Key::from(2),
            &Key::from(0),
        ),
        CmpOrdering::Less
    );
}

impl TestDir {
    fn new() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let unique = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("moli-indexeddb-test-{}-{nanos}", unique));
        fs::create_dir_all(&path).expect("test dir should be created");
        Self { path }
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn seed_database_record(manager: &mut IndexedDbManager, origin: &str, name: &str) {
    let opened = manager
        .open(OpenOptions {
            origin: origin.to_owned(),
            name: name.to_owned(),
            version: None,
        })
        .expect("open should succeed");
    if let Some(upgrade) = opened.upgrade_transaction {
        manager
            .create_object_store(upgrade, "items", ObjectStoreOptions::default())
            .expect("store should be created");
        manager
            .commit_transaction(upgrade)
            .expect("upgrade commit should succeed");
    }
    let tx = manager
        .begin_transaction(
            opened.database,
            &["items".into()],
            TransactionMode::ReadWrite,
        )
        .expect("readwrite transaction should start");
    manager
        .put(tx, "items", Some(Key::from("alpha")), b"record".to_vec())
        .expect("put should succeed");
    manager
        .commit_transaction(tx)
        .expect("record transaction should commit");
    manager
        .close_database(opened.database)
        .expect("database should close");
}

#[test]
fn open_new_database_creates_upgrade_transaction() {
    let dir = TestDir::new();
    let mut manager = IndexedDbManager::new(&dir.path).expect("manager should be created");

    let opened = manager
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("open should succeed");

    assert_eq!(
        opened.disposition,
        OpenDisposition::UpgradeNeeded {
            old_version: 0,
            new_version: 1
        }
    );
    assert!(opened.upgrade_transaction.is_some());
}

#[test]
fn in_memory_manager_does_not_persist_across_reopen() {
    let mut manager = IndexedDbManager::new_in_memory();

    let opened = manager
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "app".to_owned(),
            version: Some(1),
        })
        .expect("open should succeed");
    let upgrade = opened
        .upgrade_transaction
        .expect("upgrade transaction should exist");
    manager
        .create_object_store(
            upgrade,
            "items",
            ObjectStoreOptions {
                key_path: None,
                auto_increment: false,
            },
        )
        .expect("store should be created");
    manager
        .commit_transaction(upgrade)
        .expect("upgrade commit should succeed");
    manager
        .close_database(opened.database)
        .expect("database should close");

    let reopened = IndexedDbManager::new_in_memory()
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("reopen should succeed");

    assert_eq!(
        reopened.disposition,
        OpenDisposition::UpgradeNeeded {
            old_version: 0,
            new_version: 1
        }
    );
    assert!(reopened.upgrade_transaction.is_some());
}

#[test]
fn transaction_mode_uses_indexeddb_labels() {
    use std::str::FromStr;

    assert_eq!(
        TransactionMode::from_str("readonly"),
        Ok(TransactionMode::ReadOnly)
    );
    assert_eq!(
        TransactionMode::from_str("readwrite"),
        Ok(TransactionMode::ReadWrite)
    );
    assert_eq!(
        TransactionMode::from_str("versionchange"),
        Ok(TransactionMode::VersionChange)
    );
    let readonly: &'static str = TransactionMode::ReadOnly.into();
    let readwrite: &'static str = TransactionMode::ReadWrite.into();
    let versionchange: &'static str = TransactionMode::VersionChange.into();
    assert_eq!(readonly, "readonly");
    assert_eq!(readwrite, "readwrite");
    assert_eq!(versionchange, "versionchange");
    assert!(TransactionMode::from_str("read-write").is_err());
}

#[test]
fn upgrade_transaction_can_create_store_and_persist() {
    let dir = TestDir::new();
    let mut manager = IndexedDbManager::new(&dir.path).expect("manager should be created");

    let opened = manager
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "app".to_owned(),
            version: Some(3),
        })
        .expect("open should succeed");
    let upgrade = opened
        .upgrade_transaction
        .expect("upgrade transaction should exist");

    manager
        .create_object_store(
            upgrade,
            "items",
            ObjectStoreOptions {
                key_path: None,
                auto_increment: true,
            },
        )
        .expect("store should be created");
    manager
        .commit_transaction(upgrade)
        .expect("upgrade commit should succeed");
    manager
        .close_database(opened.database)
        .expect("database should close");

    let reopened = IndexedDbManager::new(&dir.path)
        .expect("manager should be recreated")
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("reopen should succeed");

    assert_eq!(reopened.disposition, OpenDisposition::Existing);
    assert!(reopened.upgrade_transaction.is_none());
}

#[test]
fn upgrade_transaction_can_persist_index_metadata() {
    let dir = TestDir::new();
    let mut manager = IndexedDbManager::new(&dir.path).expect("manager should be created");

    let opened = manager
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "app".to_owned(),
            version: Some(1),
        })
        .expect("open should succeed");
    let upgrade = opened
        .upgrade_transaction
        .expect("upgrade transaction should exist");
    manager
        .create_object_store(upgrade, "items", ObjectStoreOptions::default())
        .expect("store should be created");
    let index = manager
        .create_index(
            upgrade,
            "items",
            "by-id",
            IndexOptions {
                key_path: KeyPath::from("id"),
                unique: true,
                multi_entry: false,
            },
        )
        .expect("index should be created");

    assert_eq!(
        index,
        IndexInfo {
            name: "by-id".into(),
            key_path: KeyPath::from("id"),
            unique: true,
            multi_entry: false,
        }
    );

    manager
        .commit_transaction(upgrade)
        .expect("upgrade commit should succeed");

    let store_info = manager
        .object_store_info(opened.database, "items")
        .expect("store info should exist");
    assert_eq!(store_info.index_names, vec![IndexedDbName::from("by-id")]);

    let index_info = manager
        .index_info(opened.database, "items", "by-id")
        .expect("index info should exist");
    assert_eq!(index_info, index);

    manager
        .close_database(opened.database)
        .expect("database should close");

    let mut reopened_manager = IndexedDbManager::new(&dir.path).expect("manager should reopen");
    let reopened = reopened_manager
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("reopen should succeed");

    let store_info = reopened_manager
        .object_store_info(reopened.database, "items")
        .expect("store info should still exist");
    assert_eq!(store_info.index_names, vec![IndexedDbName::from("by-id")]);
}

#[test]
fn index_key_path_may_be_empty_string() {
    let dir = TestDir::new();
    let mut manager = IndexedDbManager::new(&dir.path).expect("manager should be created");

    let opened = manager
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "app".to_owned(),
            version: Some(1),
        })
        .expect("open should succeed");
    let upgrade = opened
        .upgrade_transaction
        .expect("upgrade transaction should exist");
    manager
        .create_object_store(upgrade, "items", ObjectStoreOptions::default())
        .expect("store should be created");
    let index = manager
        .create_index(
            upgrade,
            "items",
            "by-value",
            IndexOptions {
                key_path: KeyPath::from(""),
                unique: false,
                multi_entry: false,
            },
        )
        .expect("empty string index keyPath should be allowed");

    assert_eq!(index.key_path, KeyPath::from(""));
}

#[test]
fn readwrite_transaction_can_store_and_reload_records() {
    let dir = TestDir::new();
    let mut manager = IndexedDbManager::new(&dir.path).expect("manager should be created");

    let opened = manager
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("open should succeed");
    let upgrade = opened
        .upgrade_transaction
        .expect("upgrade transaction should exist");
    manager
        .create_object_store(upgrade, "items", ObjectStoreOptions::default())
        .expect("store should be created");
    manager
        .commit_transaction(upgrade)
        .expect("upgrade commit should succeed");

    let tx = manager
        .begin_transaction(
            opened.database,
            &["items".into()],
            TransactionMode::ReadWrite,
        )
        .expect("readwrite transaction should start");
    manager
        .put(tx, "items", Some(Key::from("alpha")), b"one".to_vec())
        .expect("put should succeed");
    manager
        .commit_transaction(tx)
        .expect("transaction should commit");
    manager
        .close_database(opened.database)
        .expect("database should close");

    let mut manager = IndexedDbManager::new(&dir.path).expect("manager should reopen");
    let reopened = manager
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("reopen should succeed");
    let tx = manager
        .begin_transaction(
            reopened.database,
            &["items".into()],
            TransactionMode::ReadOnly,
        )
        .expect("readonly transaction should start");
    let value = manager
        .get(tx, "items", &Key::from("alpha"))
        .expect("get should succeed");
    assert_eq!(value, RequestOutcome::Value(Some(b"one".to_vec().into())));
}

#[test]
fn external_blob_file_and_file_system_handle_objects_persist_with_their_record() {
    let dir = TestDir::new();
    let origin = "https://external-objects.example";
    let stored_value = IndexedDbValue::new(
        vec![0xFF, 0x0F],
        vec![
            IndexedDbExternalObject::Blob {
                bytes: b"blob bytes".to_vec(),
                mime_type: "text/plain".to_owned(),
            },
            IndexedDbExternalObject::File {
                bytes: b"file bytes".to_vec(),
                mime_type: "text/custom".to_owned(),
                name: "note.txt".to_owned(),
                last_modified: 123.0,
            },
            IndexedDbExternalObject::FileSystemHandle {
                kind: crate::IndexedDbFileSystemHandleKind::File,
                bucket: crate::IndexedDbFileSystemHandleBucket::Named { bucket_id: 42 },
                path: vec!["directory".to_owned(), "note.txt".to_owned()],
            },
        ],
    );
    let mut manager = IndexedDbManager::new(&dir.path).expect("manager should be created");
    let opened = manager
        .open(OpenOptions {
            origin: origin.to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("open should succeed");
    let upgrade = opened
        .upgrade_transaction
        .expect("upgrade transaction should exist");
    manager
        .create_object_store(upgrade, "items", ObjectStoreOptions::default())
        .expect("store should be created");
    manager
        .commit_transaction(upgrade)
        .expect("upgrade should commit");
    let write = manager
        .begin_transaction(
            opened.database,
            &["items".into()],
            TransactionMode::ReadWrite,
        )
        .expect("write transaction should start");
    manager
        .put(
            write,
            "items",
            Some(Key::from("external")),
            stored_value.clone(),
        )
        .expect("external value should be stored");
    manager
        .commit_transaction(write)
        .expect("write transaction should commit");
    manager
        .close_database(opened.database)
        .expect("database should close");

    let mut manager = IndexedDbManager::new(&dir.path).expect("manager should reopen");
    let reopened = manager
        .open(OpenOptions {
            origin: origin.to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("database should reopen");
    let read = manager
        .begin_transaction(
            reopened.database,
            &["items".into()],
            TransactionMode::ReadOnly,
        )
        .expect("read transaction should start");
    assert_eq!(
        manager
            .get(read, "items", &Key::from("external"))
            .expect("stored external value should be readable"),
        RequestOutcome::Value(Some(stored_value))
    );
}

#[test]
fn write_quota_rejection_rolls_back_working_copy() {
    let mut manager = IndexedDbManager::new_in_memory();
    let origin = "https://quota.example";
    let opened = manager
        .open(OpenOptions {
            origin: origin.to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("open should succeed");
    let upgrade = opened
        .upgrade_transaction
        .expect("upgrade transaction should exist");
    manager
        .create_object_store(upgrade, "items", ObjectStoreOptions::default())
        .expect("store should be created");
    manager
        .commit_transaction(upgrade)
        .expect("upgrade commit should succeed");

    let tx = manager
        .begin_transaction(
            opened.database,
            &["items".into()],
            TransactionMode::ReadWrite,
        )
        .expect("readwrite transaction should start");
    manager
        .put(tx, "items", Some(Key::from("alpha")), b"one".to_vec())
        .expect("initial put should succeed");
    manager
        .commit_transaction(tx)
        .expect("initial transaction should commit");
    let before = manager
        .origin_usage_bytes(origin)
        .expect("usage should be readable");

    let tx = manager
        .begin_transaction(
            opened.database,
            &["items".into()],
            TransactionMode::ReadWrite,
        )
        .expect("readwrite transaction should start");
    let error = manager
        .put_with_quota(
            tx,
            "items",
            Some(Key::from("alpha")),
            vec![b'x'; 4096],
            IndexedDbQuotaCheck {
                quota: before + 10,
                non_indexed_db_usage: 0,
            },
        )
        .expect_err("oversized replacement should exceed quota");
    assert!(matches!(error, IndexedDbError::QuotaExceeded { .. }));
    let value = manager
        .get(tx, "items", &Key::from("alpha"))
        .expect("get should succeed after quota rejection");
    assert_eq!(value, RequestOutcome::Value(Some(b"one".to_vec().into())));
    manager
        .commit_transaction(tx)
        .expect("rollback-preserved transaction should commit");
    assert_eq!(
        manager
            .origin_usage_bytes(origin)
            .expect("usage should be readable"),
        before
    );
}

#[test]
fn external_blob_bytes_participate_in_quota_and_rollback() {
    let mut manager = IndexedDbManager::new_in_memory();
    let origin = "https://external-quota.example";
    let opened = manager
        .open(OpenOptions {
            origin: origin.to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("open should succeed");
    let upgrade = opened
        .upgrade_transaction
        .expect("upgrade transaction should exist");
    manager
        .create_object_store(upgrade, "items", ObjectStoreOptions::default())
        .expect("store should be created");
    manager
        .commit_transaction(upgrade)
        .expect("upgrade should commit");
    let before = manager
        .origin_usage_bytes(origin)
        .expect("usage should be readable");

    let write = manager
        .begin_transaction(
            opened.database,
            &["items".into()],
            TransactionMode::ReadWrite,
        )
        .expect("write transaction should start");
    let value = IndexedDbValue::new(
        vec![1],
        vec![IndexedDbExternalObject::Blob {
            bytes: vec![b'x'; 4096],
            mime_type: "application/octet-stream".to_owned(),
        }],
    );
    let error = manager
        .put_with_quota(
            write,
            "items",
            Some(Key::from("blob")),
            value,
            IndexedDbQuotaCheck {
                quota: before + 100,
                non_indexed_db_usage: 0,
            },
        )
        .expect_err("external Blob bytes should exceed quota");
    assert!(matches!(error, IndexedDbError::QuotaExceeded { .. }));
    assert_eq!(
        manager
            .get(write, "items", &Key::from("blob"))
            .expect("quota rollback should leave the record absent"),
        RequestOutcome::Value(None)
    );
}

#[test]
fn transaction_commit_rechecks_aggregate_quota_without_publishing_working_copy() {
    let mut manager = IndexedDbManager::new_in_memory();
    let origin = "https://commit-quota.example";
    let opened = manager
        .open(OpenOptions {
            origin: origin.to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("open should succeed");
    let upgrade = opened
        .upgrade_transaction
        .expect("upgrade transaction should exist");
    manager
        .create_object_store(upgrade, "items", ObjectStoreOptions::default())
        .expect("store should be created");
    manager
        .commit_transaction(upgrade)
        .expect("upgrade should commit");
    let committed_before = manager
        .origin_usage_bytes(origin)
        .expect("usage should be readable");

    let write = manager
        .begin_transaction(
            opened.database,
            &["items".into()],
            TransactionMode::ReadWrite,
        )
        .expect("write transaction should start");
    manager
        .put(write, "items", Some(Key::from("large")), vec![b'x'; 4096])
        .expect("working-copy write should succeed before commit gate");
    let error = manager
        .commit_transaction_with_quota(
            write,
            IndexedDbQuotaCheck {
                quota: committed_before + 64,
                non_indexed_db_usage: 0,
            },
        )
        .expect_err("commit should reject the oversized working copy");
    assert!(matches!(error, IndexedDbError::QuotaExceeded { .. }));
    assert_eq!(
        manager
            .origin_usage_bytes(origin)
            .expect("committed usage should remain readable"),
        committed_before
    );

    let read = manager
        .begin_transaction(
            opened.database,
            &["items".into()],
            TransactionMode::ReadOnly,
        )
        .expect("read transaction should start");
    assert_eq!(
        manager
            .get(read, "items", &Key::from("large"))
            .expect("read should succeed"),
        RequestOutcome::Value(None)
    );
}

#[test]
fn origin_usage_tracks_committed_metadata_and_record_bytes() {
    let mut manager = IndexedDbManager::new_in_memory();
    let origin = "https://usage.example";

    assert_eq!(
        manager
            .origin_usage_bytes(origin)
            .expect("initial usage should be readable"),
        0
    );

    let opened = manager
        .open(OpenOptions {
            origin: origin.to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("open should succeed");
    let upgrade = opened
        .upgrade_transaction
        .expect("upgrade transaction should exist");
    manager
        .create_object_store(upgrade, "items", ObjectStoreOptions::default())
        .expect("store should be created");
    manager
        .commit_transaction(upgrade)
        .expect("upgrade commit should succeed");
    let metadata_usage = manager
        .origin_usage_bytes(origin)
        .expect("metadata usage should be readable");
    assert!(
        metadata_usage > 0,
        "an empty committed object store should contribute metadata usage"
    );

    let tx = manager
        .begin_transaction(
            opened.database,
            &["items".into()],
            TransactionMode::ReadWrite,
        )
        .expect("readwrite transaction should start");
    manager
        .put(tx, "items", Some(Key::from("alpha")), b"first".to_vec())
        .expect("put should succeed");
    assert_eq!(
        manager
            .origin_usage_bytes(origin)
            .expect("uncommitted usage should be readable"),
        metadata_usage,
        "usage should only expose committed data"
    );
    manager
        .commit_transaction(tx)
        .expect("transaction should commit");
    let first_record_usage = manager
        .origin_usage_bytes(origin)
        .expect("committed usage should be readable");
    assert!(first_record_usage > metadata_usage);

    let tx = manager
        .begin_transaction(
            opened.database,
            &["items".into()],
            TransactionMode::ReadWrite,
        )
        .expect("replace transaction should start");
    manager
        .put(tx, "items", Some(Key::from("alpha")), b"xx".to_vec())
        .expect("replace should succeed");
    manager
        .commit_transaction(tx)
        .expect("replace transaction should commit");
    let replacement_usage = manager
        .origin_usage_bytes(origin)
        .expect("replacement usage should be readable");
    assert!(replacement_usage > metadata_usage);
    assert!(replacement_usage < first_record_usage);
    assert_eq!(
        manager
            .origin_usage_bytes("https://other.example")
            .expect("other origin usage should be readable"),
        0
    );
}

#[test]
fn origin_prefix_usage_sums_matching_storage_key_owners() {
    let dir = TestDir::new();
    let prefix = "storage-key:v1;origin=https://usage.example;top-level-site=";
    let first_party = format!("{prefix}https://usage.example");
    let partitioned = format!("{prefix}https://top.example");
    let sibling =
        "storage-key:v1;origin=https://sibling.example;top-level-site=https://usage.example";

    {
        let mut manager = IndexedDbManager::new(&dir.path).expect("manager should be created");
        seed_database_record(&mut manager, &first_party, "first-party");
        seed_database_record(&mut manager, &partitioned, "partitioned");
        seed_database_record(&mut manager, sibling, "sibling");
    }

    let mut manager = IndexedDbManager::new(&dir.path).expect("manager should reopen");
    let total = manager
        .origins_with_prefix_usage_bytes(prefix)
        .expect("prefix usage should be readable");
    let sibling_usage = manager
        .origin_usage_bytes(sibling)
        .expect("sibling usage should be readable");

    assert!(total > 0);
    assert!(sibling_usage > 0);
    assert_eq!(
        manager
            .origins_with_prefix_usage_bytes(
                "storage-key:v1;origin=https://missing.example;top-level-site="
            )
            .expect("missing prefix usage should be readable"),
        0
    );
}

#[test]
fn clear_origins_with_prefix_removes_only_matching_storage_key_owners() {
    let dir = TestDir::new();
    let prefix = "storage-key:v1;origin=https://clear.example;top-level-site=";
    let first_party = format!("{prefix}https://clear.example");
    let partitioned = format!("{prefix}https://top.example");
    let sibling =
        "storage-key:v1;origin=https://sibling.example;top-level-site=https://clear.example";

    {
        let mut manager = IndexedDbManager::new(&dir.path).expect("manager should be created");
        seed_database_record(&mut manager, &first_party, "first-party");
        seed_database_record(&mut manager, &partitioned, "partitioned");
        seed_database_record(&mut manager, sibling, "sibling");
    }

    let mut manager = IndexedDbManager::new(&dir.path).expect("manager should reopen");
    manager
        .clear_origins_with_prefix(prefix)
        .expect("prefix clear should succeed");

    assert_eq!(
        manager
            .origin_usage_bytes(&first_party)
            .expect("first-party usage should be readable"),
        0
    );
    assert_eq!(
        manager
            .origin_usage_bytes(&partitioned)
            .expect("partitioned usage should be readable"),
        0
    );
    assert!(
        manager
            .origin_usage_bytes(sibling)
            .expect("sibling usage should be readable")
            > 0
    );
}

#[test]
fn clear_origins_with_prefix_removes_hash_named_storage_key_owners() {
    let dir = TestDir::new();
    let prefix = "storage-key:v1;origin=https://clear-long.example;top-level-site=";
    let first_party = format!("{prefix}https://{}", "a".repeat(120));
    let partitioned = format!("{prefix}https://{}", "b".repeat(120));
    let sibling = format!(
        "storage-key:v1;origin=https://sibling.example;top-level-site=https://{}",
        "c".repeat(120)
    );

    assert!(origin_file_stem(&first_party).starts_with("h-"));
    assert!(origin_file_stem(&partitioned).starts_with("h-"));
    assert!(origin_file_stem(&sibling).starts_with("h-"));

    {
        let mut manager = IndexedDbManager::new(&dir.path).expect("manager should be created");
        seed_database_record(&mut manager, &first_party, "first-party");
        seed_database_record(&mut manager, &partitioned, "partitioned");
        seed_database_record(&mut manager, &sibling, "sibling");
    }

    assert!(origin_path(&dir.path, &first_party).exists());
    assert!(origin_path(&dir.path, &partitioned).exists());
    assert!(origin_path(&dir.path, &sibling).exists());

    let mut manager = IndexedDbManager::new(&dir.path).expect("manager should reopen");
    manager
        .clear_origins_with_prefix(prefix)
        .expect("prefix clear should succeed");

    assert!(!origin_path(&dir.path, &first_party).exists());
    assert!(!origin_path(&dir.path, &partitioned).exists());
    assert!(origin_path(&dir.path, &sibling).exists());
    assert!(
        manager
            .origin_usage_bytes(&sibling)
            .expect("sibling usage should be readable")
            > 0
    );
}

#[test]
fn migrate_origin_preserves_data_across_restart_and_is_replay_safe() {
    let dir = TestDir::new();
    let source = "bucket:v1:legacy-name-key";
    let destination = "bucket:v2:persistent-id-key:17";
    let expected_usage = {
        let mut manager = IndexedDbManager::new(&dir.path).expect("manager should be created");
        seed_database_record(&mut manager, source, "app");
        let expected = manager
            .origin_usage_bytes(source)
            .expect("source usage should load");
        manager
            .migrate_origin(source, destination)
            .expect("owner migration should succeed");
        assert_eq!(
            manager
                .origin_usage_bytes(source)
                .expect("migrated source should be empty"),
            0
        );
        assert_eq!(
            manager
                .origin_usage_bytes(destination)
                .expect("destination usage should load"),
            expected
        );
        manager
            .migrate_origin(source, destination)
            .expect("replaying a completed migration should be harmless");
        expected
    };

    let mut reopened = IndexedDbManager::new(&dir.path).expect("manager should reopen");
    assert_eq!(
        reopened
            .origin_usage_bytes(source)
            .expect("source usage should load after restart"),
        0
    );
    assert_eq!(
        reopened
            .origin_usage_bytes(destination)
            .expect("destination usage should load after restart"),
        expected_usage
    );
}

#[test]
fn readwrite_transaction_can_list_keys() {
    let dir = TestDir::new();
    let mut manager = IndexedDbManager::new(&dir.path).expect("manager should be created");

    let opened = manager
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("open should succeed");
    let upgrade = opened
        .upgrade_transaction
        .expect("upgrade transaction should exist");
    manager
        .create_object_store(upgrade, "items", ObjectStoreOptions::default())
        .expect("store should be created");
    manager
        .commit_transaction(upgrade)
        .expect("upgrade commit should succeed");

    let tx = manager
        .begin_transaction(
            opened.database,
            &["items".into()],
            TransactionMode::ReadWrite,
        )
        .expect("readwrite transaction should start");
    manager
        .put(tx, "items", Some(Key::from("beta")), b"two".to_vec())
        .expect("put should succeed");
    manager
        .put(tx, "items", Some(Key::from("alpha")), b"one".to_vec())
        .expect("put should succeed");

    let keys = manager
        .get_all_keys(tx, "items")
        .expect("get_all_keys should succeed");
    assert_eq!(
        keys,
        RequestOutcome::Keys(vec![Key::from("alpha"), Key::from("beta")])
    );
}

#[test]
fn mixed_keys_sort_in_indexeddb_order() {
    let dir = TestDir::new();
    let mut manager = IndexedDbManager::new(&dir.path).expect("manager should be created");

    let opened = manager
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("open should succeed");
    let upgrade = opened
        .upgrade_transaction
        .expect("upgrade transaction should exist");
    manager
        .create_object_store(upgrade, "items", ObjectStoreOptions::default())
        .expect("store should be created");
    manager
        .commit_transaction(upgrade)
        .expect("upgrade commit should succeed");

    let tx = manager
        .begin_transaction(
            opened.database,
            &["items".into()],
            TransactionMode::ReadWrite,
        )
        .expect("readwrite transaction should start");
    manager
        .put(tx, "items", Some(Key::from(10)), b"ten".to_vec())
        .expect("integer put should succeed");
    manager
        .put(tx, "items", Some(Key::from("alpha")), b"one".to_vec())
        .expect("string put should succeed");
    manager
        .put(tx, "items", Some(Key::from(2)), b"two".to_vec())
        .expect("integer put should succeed");

    let keys = manager
        .get_all_keys(tx, "items")
        .expect("get_all_keys should succeed");
    assert_eq!(
        keys,
        RequestOutcome::Keys(vec![Key::from(2), Key::from(10), Key::from("alpha"),])
    );
}

#[test]
fn aborted_transaction_does_not_persist_changes() {
    let dir = TestDir::new();
    let mut manager = IndexedDbManager::new(&dir.path).expect("manager should be created");

    let opened = manager
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("open should succeed");
    let upgrade = opened
        .upgrade_transaction
        .expect("upgrade transaction should exist");
    manager
        .create_object_store(upgrade, "items", ObjectStoreOptions::default())
        .expect("store should be created");
    manager
        .commit_transaction(upgrade)
        .expect("upgrade commit should succeed");

    let tx = manager
        .begin_transaction(
            opened.database,
            &["items".into()],
            TransactionMode::ReadWrite,
        )
        .expect("readwrite transaction should start");
    manager
        .put(tx, "items", Some(Key::from("alpha")), b"one".to_vec())
        .expect("put should succeed");
    manager.abort_transaction(tx).expect("abort should succeed");

    let check = manager
        .begin_transaction(
            opened.database,
            &["items".into()],
            TransactionMode::ReadOnly,
        )
        .expect("readonly transaction should start");
    let value = manager
        .get(check, "items", &Key::from("alpha"))
        .expect("get should succeed");
    assert_eq!(value, RequestOutcome::Value(None));
}

#[test]
fn delete_database_removes_persisted_state() {
    let dir = TestDir::new();
    let mut manager = IndexedDbManager::new(&dir.path).expect("manager should be created");

    let opened = manager
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("open should succeed");
    let upgrade = opened
        .upgrade_transaction
        .expect("upgrade transaction should exist");
    manager
        .create_object_store(upgrade, "items", ObjectStoreOptions::default())
        .expect("store should be created");
    manager
        .commit_transaction(upgrade)
        .expect("upgrade commit should succeed");
    manager
        .close_database(opened.database)
        .expect("database should close");
    manager
        .delete_database("https://example.com", "app")
        .expect("delete should succeed");

    let reopened = IndexedDbManager::new(&dir.path)
        .expect("manager should reopen")
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("open should succeed");

    assert_eq!(
        reopened.disposition,
        OpenDisposition::UpgradeNeeded {
            old_version: 0,
            new_version: 1
        }
    );
}

#[test]
fn databases_lists_committed_name_version_snapshot() {
    let dir = TestDir::new();
    let mut manager = IndexedDbManager::new(&dir.path).expect("manager should be created");

    let beta = manager
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "beta".to_owned(),
            version: Some(2),
        })
        .expect("beta open should succeed");
    manager
        .commit_transaction(beta.upgrade_transaction.expect("beta upgrade tx"))
        .expect("beta upgrade commit should succeed");
    let alpha = manager
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "alpha".to_owned(),
            version: None,
        })
        .expect("alpha open should succeed");
    manager
        .commit_transaction(alpha.upgrade_transaction.expect("alpha upgrade tx"))
        .expect("alpha upgrade commit should succeed");

    let infos = manager
        .databases("https://example.com")
        .expect("database list should be readable");
    assert_eq!(
        infos,
        vec![
            DatabaseNameAndVersion {
                name: "alpha".to_owned(),
                version: 1,
            },
            DatabaseNameAndVersion {
                name: "beta".to_owned(),
                version: 2,
            },
        ]
    );

    let mut reopened = IndexedDbManager::new(&dir.path).expect("manager should reopen");
    let infos = reopened
        .databases("https://example.com")
        .expect("persisted database list should be readable");
    assert_eq!(
        infos
            .into_iter()
            .map(|info| format!("{}:{}", info.name, info.version))
            .collect::<Vec<_>>(),
        vec!["alpha:1", "beta:2"]
    );
}

#[test]
fn clear_origin_removes_persisted_state_and_keeps_other_origins() {
    let dir = TestDir::new();
    let mut manager = IndexedDbManager::new(&dir.path).expect("manager should be created");

    let first = manager
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("first open should succeed");
    let first_upgrade = first
        .upgrade_transaction
        .expect("first upgrade transaction should exist");
    manager
        .create_object_store(first_upgrade, "items", ObjectStoreOptions::default())
        .expect("first store should be created");
    manager
        .commit_transaction(first_upgrade)
        .expect("first upgrade commit should succeed");

    let second = manager
        .open(OpenOptions {
            origin: "https://other.example".to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("second open should succeed");
    let second_upgrade = second
        .upgrade_transaction
        .expect("second upgrade transaction should exist");
    manager
        .create_object_store(second_upgrade, "items", ObjectStoreOptions::default())
        .expect("second store should be created");
    manager
        .commit_transaction(second_upgrade)
        .expect("second upgrade commit should succeed");

    manager
        .clear_origin("https://example.com")
        .expect("origin clear should succeed");

    let mut reopened = IndexedDbManager::new(&dir.path).expect("manager should reopen from disk");
    let cleared = reopened
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("cleared origin open should succeed");
    assert_eq!(
        cleared.disposition,
        OpenDisposition::UpgradeNeeded {
            old_version: 0,
            new_version: 1
        }
    );

    let kept = reopened
        .open(OpenOptions {
            origin: "https://other.example".to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("other origin open should succeed");
    assert_eq!(kept.disposition, OpenDisposition::Existing);
}

#[test]
fn readonly_transaction_rejects_writes() {
    let dir = TestDir::new();
    let mut manager = IndexedDbManager::new(&dir.path).expect("manager should be created");

    let opened = manager
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("open should succeed");
    let upgrade = opened
        .upgrade_transaction
        .expect("upgrade transaction should exist");
    manager
        .create_object_store(upgrade, "items", ObjectStoreOptions::default())
        .expect("store should be created");
    manager
        .commit_transaction(upgrade)
        .expect("upgrade commit should succeed");

    let tx = manager
        .begin_transaction(
            opened.database,
            &["items".into()],
            TransactionMode::ReadOnly,
        )
        .expect("readonly transaction should start");

    let error = manager
        .put(tx, "items", Some(Key::from("alpha")), b"one".to_vec())
        .expect_err("readonly put should fail");
    assert!(matches!(error, IndexedDbError::ReadOnly(_)));
}

#[test]
fn transaction_scope_is_enforced_per_object_store() {
    let dir = TestDir::new();
    let mut manager = IndexedDbManager::new(&dir.path).expect("manager should be created");

    let opened = manager
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("open should succeed");
    let upgrade = opened
        .upgrade_transaction
        .expect("upgrade transaction should exist");
    manager
        .create_object_store(upgrade, "items", ObjectStoreOptions::default())
        .expect("items store should be created");
    manager
        .create_object_store(upgrade, "other", ObjectStoreOptions::default())
        .expect("other store should be created");
    manager
        .commit_transaction(upgrade)
        .expect("upgrade commit should succeed");

    let tx = manager
        .begin_transaction(
            opened.database,
            &["items".into()],
            TransactionMode::ReadWrite,
        )
        .expect("readwrite transaction should start");

    let error = manager
        .put(tx, "other", Some(Key::from("alpha")), b"one".to_vec())
        .expect_err("out-of-scope store write should fail");
    assert!(matches!(error, IndexedDbError::InvalidState(_)));
}

#[test]
fn delete_database_rejects_open_connections() {
    let dir = TestDir::new();
    let mut manager = IndexedDbManager::new(&dir.path).expect("manager should be created");

    let opened = manager
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("open should succeed");
    let upgrade = opened
        .upgrade_transaction
        .expect("upgrade transaction should exist");
    manager
        .create_object_store(upgrade, "items", ObjectStoreOptions::default())
        .expect("store should be created");
    manager
        .commit_transaction(upgrade)
        .expect("upgrade commit should succeed");

    let error = manager
        .delete_database("https://example.com", "app")
        .expect_err("delete should fail while handle remains open");
    assert!(matches!(error, IndexedDbError::InvalidState(_)));
}

#[test]
fn aborted_upgrade_transaction_does_not_publish_schema() {
    let dir = TestDir::new();
    let mut manager = IndexedDbManager::new(&dir.path).expect("manager should be created");

    let opened = manager
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "app".to_owned(),
            version: Some(2),
        })
        .expect("open should succeed");
    let upgrade = opened
        .upgrade_transaction
        .expect("upgrade transaction should exist");
    manager
        .create_object_store(upgrade, "items", ObjectStoreOptions::default())
        .expect("store should be created");
    manager
        .abort_transaction(upgrade)
        .expect("abort should succeed");
    manager
        .close_database(opened.database)
        .expect("database should close");

    let reopened = IndexedDbManager::new(&dir.path)
        .expect("manager should reopen")
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("reopen should succeed");

    assert_eq!(
        reopened.disposition,
        OpenDisposition::UpgradeNeeded {
            old_version: 0,
            new_version: 1
        }
    );
}

#[test]
fn upgrade_can_delete_object_store_before_commit() {
    let dir = TestDir::new();
    let mut manager = IndexedDbManager::new(&dir.path).expect("manager should be created");

    let opened = manager
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("open should succeed");
    let upgrade = opened
        .upgrade_transaction
        .expect("upgrade transaction should exist");
    manager
        .create_object_store(upgrade, "items", ObjectStoreOptions::default())
        .expect("store should be created");
    manager
        .create_object_store(upgrade, "temp", ObjectStoreOptions::default())
        .expect("temp store should be created");
    manager
        .delete_object_store(upgrade, "temp")
        .expect("temp store should be deleted");
    manager
        .commit_transaction(upgrade)
        .expect("upgrade commit should succeed");

    let info = manager
        .database_info(opened.database)
        .expect("database info should be readable");
    assert_eq!(info.object_store_names, vec![IndexedDbName::from("items")]);

    let error = manager
        .object_store_info(opened.database, "temp")
        .expect_err("deleted store should be absent");
    assert!(matches!(error, IndexedDbError::NotFound(_)));
}

#[test]
fn distinct_origins_persist_to_distinct_storage_files() {
    let dir = TestDir::new();
    let mut manager = IndexedDbManager::new(&dir.path).expect("manager should be created");

    let first = manager
        .open(OpenOptions {
            origin: "https://a:b".to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("first open should succeed");
    let first_upgrade = first
        .upgrade_transaction
        .expect("first upgrade transaction should exist");
    manager
        .create_object_store(first_upgrade, "alpha", ObjectStoreOptions::default())
        .expect("alpha store should be created");
    manager
        .commit_transaction(first_upgrade)
        .expect("first commit should succeed");
    manager
        .close_database(first.database)
        .expect("first database should close");

    let second = manager
        .open(OpenOptions {
            origin: "https://a/b".to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("second open should succeed");
    let second_upgrade = second
        .upgrade_transaction
        .expect("second upgrade transaction should exist");
    manager
        .create_object_store(second_upgrade, "beta", ObjectStoreOptions::default())
        .expect("beta store should be created");
    manager
        .commit_transaction(second_upgrade)
        .expect("second commit should succeed");
    manager
        .close_database(second.database)
        .expect("second database should close");

    let mut reopened = IndexedDbManager::new(&dir.path).expect("manager should reopen");
    let first = reopened
        .open(OpenOptions {
            origin: "https://a:b".to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("first reopen should succeed");
    let second = reopened
        .open(OpenOptions {
            origin: "https://a/b".to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("second reopen should succeed");

    assert_eq!(
        reopened
            .database_info(first.database)
            .expect("first info should exist")
            .object_store_names,
        vec![IndexedDbName::from("alpha")]
    );
    assert_eq!(
        reopened
            .database_info(second.database)
            .expect("second info should exist")
            .object_store_names,
        vec![IndexedDbName::from("beta")]
    );
}

#[test]
fn failed_open_does_not_block_later_delete_database() {
    let dir = TestDir::new();
    let mut manager = IndexedDbManager::new(&dir.path).expect("manager should be created");

    let opened = manager
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "app".to_owned(),
            version: Some(2),
        })
        .expect("open should succeed");
    let upgrade = opened
        .upgrade_transaction
        .expect("upgrade transaction should exist");
    manager
        .create_object_store(upgrade, "items", ObjectStoreOptions::default())
        .expect("store should be created");
    manager
        .commit_transaction(upgrade)
        .expect("upgrade commit should succeed");
    manager
        .close_database(opened.database)
        .expect("database should close");

    let error = manager
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "app".to_owned(),
            version: Some(1),
        })
        .expect_err("lower version open should fail");
    assert!(matches!(error, IndexedDbError::Version(_)));

    manager
        .delete_database("https://example.com", "app")
        .expect("delete should still succeed after failed open");
}

#[test]
fn generated_key_preview_and_failed_quota_write_leave_generator_unchanged() {
    let dir = TestDir::new();
    let mut manager = IndexedDbManager::new(&dir.path).unwrap();
    let opened = manager
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "key-preview".to_owned(),
            version: None,
        })
        .unwrap();
    let tx = opened.upgrade_transaction.unwrap();
    manager
        .create_object_store(
            tx,
            "items",
            ObjectStoreOptions {
                key_path: None,
                auto_increment: true,
            },
        )
        .unwrap();
    assert_eq!(
        manager.next_generated_key(tx, "items").unwrap(),
        Key::from(1)
    );
    assert_eq!(
        manager.next_generated_key(tx, "items").unwrap(),
        Key::from(1)
    );
    assert_eq!(
        manager
            .put(tx, "items", Some(Key::from(1)), vec![1])
            .unwrap(),
        Key::from(1)
    );
    let error = manager
        .put_with_quota(
            tx,
            "items",
            Some(Key::from(20)),
            vec![2],
            IndexedDbQuotaCheck {
                quota: 0,
                non_indexed_db_usage: 0,
            },
        )
        .unwrap_err();
    assert!(matches!(error, IndexedDbError::QuotaExceeded { .. }));
    assert_eq!(
        manager.next_generated_key(tx, "items").unwrap(),
        Key::from(2)
    );
    assert_eq!(
        manager.put(tx, "items", None, vec![3]).unwrap(),
        Key::from(2)
    );
    manager.commit_transaction(tx).unwrap();
    let tx = manager
        .begin_transaction(
            opened.database,
            &["items".into()],
            TransactionMode::ReadWrite,
        )
        .unwrap();
    assert_eq!(
        manager.next_generated_key(tx, "items").unwrap(),
        Key::from(3)
    );
    manager.abort_transaction(tx).unwrap();
}

#[test]
fn auto_increment_rejects_exhausted_generator_range() {
    let dir = TestDir::new();
    let mut manager = IndexedDbManager::new(&dir.path).expect("manager should be created");

    let opened = manager
        .open(OpenOptions {
            origin: "https://example.com".to_owned(),
            name: "app".to_owned(),
            version: None,
        })
        .expect("open should succeed");
    let upgrade = opened
        .upgrade_transaction
        .expect("upgrade transaction should exist");
    manager
        .create_object_store(
            upgrade,
            "items",
            ObjectStoreOptions {
                key_path: None,
                auto_increment: true,
            },
        )
        .expect("store should be created");
    manager
        .commit_transaction(upgrade)
        .expect("upgrade commit should succeed");

    let tx = manager
        .begin_transaction(
            opened.database,
            &["items".into()],
            TransactionMode::ReadWrite,
        )
        .expect("readwrite transaction should start");
    let store = transaction_store_mut(
        manager
            .transactions
            .get_mut(&tx)
            .expect("transaction should exist"),
        &"items".into(),
    )
    .expect("store should exist");
    store.auto_increment_counter = MAX_AUTO_INCREMENT_KEY;

    let error = manager
        .generate_key(tx, "items")
        .expect_err("generate_key should fail once the safe integer range is exhausted");
    assert!(matches!(error, IndexedDbError::Constraint(_)));
}

#[test]
fn all_key_types_survive_backend_reopen_and_remain_distinct() {
    let dir = TestDir::new();
    let options = OpenOptions {
        origin: "https://keys.test".into(),
        name: "keys".into(),
        version: None,
    };
    let mut keys = vec![
        Key::number(f64::NEG_INFINITY).unwrap(),
        Key::from(0),
        Key::number(f64::from_bits(1)).unwrap(),
        Key::number(1.25).unwrap(),
        Key::number(f64::MAX).unwrap(),
        Key::number(f64::INFINITY).unwrap(),
        Key::Date(0),
        Key::String(vec![0xd800]),
        Key::String(vec![0xdc00]),
        Key::from("\u{fffd}"),
        Key::Binary(vec![0, 255]),
        Key::Array(vec![Key::Date(42), Key::Binary(vec![7])]),
    ];
    let mut deep = Key::from(1);
    for _ in 0..100 {
        deep = Key::Array(vec![deep]);
    }
    keys.push(deep);
    {
        let mut manager = IndexedDbManager::new(&dir.path).unwrap();
        let opened = manager.open(options.clone()).unwrap();
        let tx = opened.upgrade_transaction.unwrap();
        manager
            .create_object_store(
                tx,
                "items",
                ObjectStoreOptions {
                    key_path: None,
                    auto_increment: false,
                },
            )
            .unwrap();
        for (index, key) in keys.iter().enumerate() {
            manager
                .add(tx, "items", Some(key.clone()), vec![index as u8])
                .unwrap();
        }
        manager.commit_transaction(tx).unwrap();
        manager.close_database(opened.database).unwrap();
    }
    let mut manager = IndexedDbManager::new(&dir.path).unwrap();
    let opened = manager.open(options).unwrap();
    let tx = manager
        .begin_transaction(
            opened.database,
            &["items".into()],
            TransactionMode::ReadOnly,
        )
        .unwrap();
    assert_eq!(
        manager.get_all_keys(tx, "items").unwrap(),
        RequestOutcome::Keys(keys.clone())
    );
    for (index, key) in keys.iter().enumerate() {
        assert_eq!(
            manager.get(tx, "items", key).unwrap(),
            RequestOutcome::Value(Some(vec![index as u8].into()))
        );
    }
    manager.commit_transaction(tx).unwrap();
}

#[test]
fn generator_uses_numeric_floor_and_inclusive_2pow53_boundary() {
    let dir = TestDir::new();
    let mut manager = IndexedDbManager::new(&dir.path).unwrap();
    let opened = manager
        .open(OpenOptions {
            origin: "https://keys.test".into(),
            name: "generator".into(),
            version: None,
        })
        .unwrap();
    let tx = opened.upgrade_transaction.unwrap();
    manager
        .create_object_store(
            tx,
            "items",
            ObjectStoreOptions {
                key_path: None,
                auto_increment: true,
            },
        )
        .unwrap();
    manager
        .put(tx, "items", Some(Key::Date(1000)), vec![])
        .unwrap();
    assert_eq!(
        manager.put(tx, "items", None, vec![]).unwrap(),
        Key::from(1)
    );
    manager.put(tx, "items", Key::number(3.9), vec![]).unwrap();
    assert_eq!(
        manager.put(tx, "items", None, vec![]).unwrap(),
        Key::from(4)
    );
    let last = MAX_AUTO_INCREMENT_KEY as i64;
    manager
        .put(tx, "items", Some(Key::from(last - 1)), vec![])
        .unwrap();
    assert_eq!(
        manager.put(tx, "items", None, vec![]).unwrap(),
        Key::from(last)
    );
    assert!(matches!(
        manager.put(tx, "items", None, vec![]),
        Err(IndexedDbError::Constraint(_))
    ));
    // Exhaustion limits generated keys, not valid explicit numeric keys.
    manager
        .put(tx, "items", Key::number(f64::INFINITY), vec![])
        .unwrap();
    manager
        .put(tx, "items", Key::number(f64::NEG_INFINITY), vec![])
        .unwrap();
    manager.commit_transaction(tx).unwrap();
    let tx = manager
        .begin_transaction(
            opened.database,
            &["items".into()],
            TransactionMode::ReadWrite,
        )
        .unwrap();
    assert!(matches!(
        manager.next_generated_key(tx, "items"),
        Err(IndexedDbError::Constraint(_))
    ));
    manager.abort_transaction(tx).unwrap();
}
