use super::*;

fn open_database(manager: &mut IndexedDbManager) -> DatabaseHandle {
    let open = manager
        .open(OpenOptions {
            origin: "origin".into(),
            name: "db".into(),
            version: None,
        })
        .unwrap();
    if let Some(upgrade) = open.upgrade_transaction {
        for store in ["a", "b", "c"] {
            manager
                .create_object_store(
                    upgrade,
                    store,
                    ObjectStoreOptions {
                        auto_increment: true,
                        ..Default::default()
                    },
                )
                .unwrap();
        }
        manager.commit_transaction(upgrade).unwrap();
    }
    open.database
}

#[test]
fn disjoint_writers_and_readers_preserve_other_commits_in_memory_and_on_disk() {
    let dir = TestDir::new();
    let mut manager = IndexedDbManager::new(&dir.path).unwrap();
    let database = open_database(&mut manager);
    let first_tx = manager
        .begin_transaction(database, &["a".into()], TransactionMode::ReadWrite)
        .unwrap();
    let second_tx = manager
        .begin_transaction(database, &["b".into()], TransactionMode::ReadWrite)
        .unwrap();
    let reader_tx = manager
        .begin_transaction(database, &["c".into()], TransactionMode::ReadOnly)
        .unwrap();
    assert!(
        manager
            .begin_transaction(database, &["a".into()], TransactionMode::ReadWrite)
            .is_err()
    );
    manager.put(first_tx, "a", None, vec![1]).unwrap();
    manager.put(second_tx, "b", None, vec![2]).unwrap();
    manager.commit_transaction(first_tx).unwrap();
    manager.commit_transaction(second_tx).unwrap();
    // A readonly transaction must neither revert the writes nor fail quota.
    manager
        .commit_transaction_with_quota(
            reader_tx,
            IndexedDbQuotaCheck {
                quota: 0,
                non_indexed_db_usage: 0,
            },
        )
        .unwrap();
    for reload in [false, true] {
        if reload {
            manager = IndexedDbManager::new(&dir.path).unwrap();
        }
        let db = open_database(&mut manager);
        let tx = manager
            .begin_transaction(db, &["a".into(), "b".into()], TransactionMode::ReadWrite)
            .unwrap();
        for (store, value) in [("a", 1), ("b", 2)] {
            assert_eq!(
                manager.get(tx, store, &Key::from(1)).unwrap(),
                RequestOutcome::Value(Some(vec![value].into()))
            );
            assert_eq!(manager.put(tx, store, None, vec![3]).unwrap(), Key::from(2));
        }
        manager.abort_transaction(tx).unwrap();
        manager.close_database(db).unwrap();
    }
}

#[test]
fn quota_checks_include_new_commits_to_other_stores() {
    for at_commit in [false, true] {
        let mut manager = IndexedDbManager::new_in_memory();
        let database = open_database(&mut manager);
        let initial_usage = manager.origin_usage_bytes("origin").unwrap();
        let first = manager
            .begin_transaction(database, &["a".into()], TransactionMode::ReadWrite)
            .unwrap();
        let second = manager
            .begin_transaction(database, &["b".into()], TransactionMode::ReadWrite)
            .unwrap();
        let quota = IndexedDbQuotaCheck {
            quota: initial_usage + 180,
            non_indexed_db_usage: 0,
        };
        if at_commit {
            manager
                .put_with_quota(second, "b", None, vec![2; 100], quota)
                .unwrap();
        }
        manager.put(first, "a", None, vec![1; 100]).unwrap();
        manager.commit_transaction_with_quota(first, quota).unwrap();
        let published_usage = manager.origin_usage_bytes("origin").unwrap();
        let result = if at_commit {
            manager
                .commit_transaction_with_quota(second, quota)
                .map(|_| Key::from(0))
        } else {
            manager.put_with_quota(second, "b", None, vec![2; 100], quota)
        };
        assert!(matches!(result, Err(IndexedDbError::QuotaExceeded { .. })));
        if !at_commit {
            assert_eq!(manager.generate_key(second, "b").unwrap(), Key::from(1));
            manager.commit_transaction(second).unwrap();
        }
        assert_eq!(
            manager.origin_usage_bytes("origin").unwrap(),
            published_usage
        );
        let read = manager
            .begin_transaction(
                database,
                &["a".into(), "b".into()],
                TransactionMode::ReadOnly,
            )
            .unwrap();
        assert_eq!(
            manager.get(read, "a", &Key::from(1)).unwrap(),
            RequestOutcome::Value(Some(vec![1; 100].into()))
        );
        assert_eq!(
            manager.get(read, "b", &Key::from(1)).unwrap(),
            RequestOutcome::Value(None)
        );
        manager.commit_transaction(read).unwrap();
    }
}
