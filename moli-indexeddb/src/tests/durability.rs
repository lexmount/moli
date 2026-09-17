use super::*;
use std::io::Read;

fn database(manager: &mut IndexedDbManager, version: u64) -> OpenResult {
    manager
        .open(OpenOptions {
            origin: "origin".into(),
            name: "durability".into(),
            version: Some(version),
        })
        .unwrap()
}

fn seed(manager: &mut IndexedDbManager) -> DatabaseHandle {
    let open = database(manager, 1);
    let upgrade = open.upgrade_transaction.unwrap();
    manager
        .create_object_store(
            upgrade,
            "records",
            ObjectStoreOptions {
                auto_increment: true,
                ..Default::default()
            },
        )
        .unwrap();
    manager.put(upgrade, "records", None, vec![1]).unwrap();
    manager.commit_transaction(upgrade).unwrap();
    open.database
}

#[test]
fn persistent_commits_replace_complete_snapshots_and_reopen_with_both_durabilities() {
    for durability in [
        TransactionDurability::Relaxed,
        TransactionDurability::Strict,
    ] {
        let dir = TestDir::new();
        let mut manager = IndexedDbManager::new(&dir.path).unwrap();
        let db = seed(&mut manager);
        let path = origin_path(&dir.path, "origin");
        let old_bytes = fs::read(&path).unwrap();
        let mut old_snapshot = fs::File::open(&path).unwrap();
        let transaction = manager
            .begin_transaction(db, &["records".into()], TransactionMode::ReadWrite)
            .unwrap();
        assert_eq!(
            manager.put(transaction, "records", None, vec![2]).unwrap(),
            Key::from(2)
        );
        manager
            .commit_transaction_with_options(
                transaction,
                TransactionCommitOptions {
                    durability,
                    quota: Some(IndexedDbQuotaCheck {
                        quota: u64::MAX,
                        non_indexed_db_usage: 0,
                    }),
                },
            )
            .unwrap();
        // A reader holding the previous file must never observe truncation or
        // bytes belonging to a later commit.
        let mut retained = Vec::new();
        old_snapshot.read_to_end(&mut retained).unwrap();
        assert_eq!(retained, old_bytes);
        assert_ne!(fs::read(&path).unwrap(), old_bytes);
        assert_eq!(fs::read_dir(&dir.path).unwrap().count(), 1);
        for reload in [false, true] {
            if reload {
                manager = IndexedDbManager::new(&dir.path).unwrap();
            }
            let db = database(&mut manager, 1).database;
            let read = manager
                .begin_transaction(db, &["records".into()], TransactionMode::ReadOnly)
                .unwrap();
            for key in [1, 2] {
                assert_eq!(
                    manager.get(read, "records", &Key::from(key)).unwrap(),
                    RequestOutcome::Value(Some(vec![key as u8].into()))
                );
            }
            manager.commit_transaction(read).unwrap();
        }
    }
}

#[test]
fn failed_persistent_commits_restore_data_schema_version_and_key_generator() {
    for durability in [
        TransactionDurability::Relaxed,
        TransactionDurability::Strict,
    ] {
        for upgrade in [false, true] {
            let dir = TestDir::new();
            let mut manager = IndexedDbManager::new(&dir.path).unwrap();
            let db = seed(&mut manager);
            let tx = if upgrade {
                manager.close_database(db).unwrap();
                let open = database(&mut manager, 2);
                let tx = open.upgrade_transaction.unwrap();
                manager
                    .create_object_store(tx, "new-store", ObjectStoreOptions::default())
                    .unwrap();
                tx
            } else {
                manager
                    .begin_transaction(db, &["records".into()], TransactionMode::ReadWrite)
                    .unwrap()
            };
            manager.put(tx, "records", None, vec![9]).unwrap();
            let path = origin_path(&dir.path, "origin");
            let bytes = fs::read(&path).unwrap();
            let saved = dir.path.join("saved");
            fs::rename(&path, &saved).unwrap();
            // A directory at the destination rejects the actual replacement
            // without depending on permissions or the test user's privileges.
            fs::create_dir(&path).unwrap();
            assert!(matches!(
                manager.commit_transaction_with_options(
                    tx,
                    TransactionCommitOptions {
                        durability,
                        ..Default::default()
                    }
                ),
                Err(IndexedDbError::Io(_))
            ));
            assert!(manager.commit_transaction(tx).is_err());
            assert_eq!(fs::read(&saved).unwrap(), bytes);
            assert_eq!(fs::read_dir(&dir.path).unwrap().count(), 2);
            fs::remove_dir(&path).unwrap();
            fs::rename(&saved, &path).unwrap();
            for reload in [false, true] {
                if reload {
                    manager = IndexedDbManager::new(&dir.path).unwrap();
                }
                let db = database(&mut manager, 1).database;
                let info = manager.database_info(db).unwrap();
                assert_eq!(info.version, 1);
                assert_eq!(info.object_store_names, [IndexedDbName::from("records")]);
                let read = manager
                    .begin_transaction(db, &["records".into()], TransactionMode::ReadWrite)
                    .unwrap();
                assert_eq!(
                    manager.get(read, "records", &Key::from(2)).unwrap(),
                    RequestOutcome::Value(None)
                );
                assert_eq!(
                    manager.put(read, "records", None, vec![3]).unwrap(),
                    Key::from(2)
                );
                manager.abort_transaction(read).unwrap();
            }
        }
    }
}

#[test]
fn strict_readonly_commit_needs_neither_disk_writes_nor_quota() {
    let dir = TestDir::new();
    let mut manager = IndexedDbManager::new(&dir.path).unwrap();
    let db = seed(&mut manager);
    let tx = manager
        .begin_transaction(db, &["records".into()], TransactionMode::ReadOnly)
        .unwrap();
    let path = origin_path(&dir.path, "origin");
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    manager
        .commit_transaction_with_options(
            tx,
            TransactionCommitOptions {
                durability: TransactionDurability::Strict,
                quota: Some(IndexedDbQuotaCheck {
                    quota: 0,
                    non_indexed_db_usage: 0,
                }),
            },
        )
        .unwrap();
    assert!(path.is_dir());
}
