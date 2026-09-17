use super::*;
use std::sync::Arc;

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

fn queue(
    manager: &IndexedDbManager,
    database: DatabaseHandle,
    stores: &[&str],
    mode: TransactionMode,
) -> TransactionRequestLease {
    manager
        .queue_transaction_start(
            database,
            &stores.iter().map(|s| (*s).into()).collect::<Vec<_>>(),
            mode,
            Arc::new(|| {}),
        )
        .unwrap()
}

#[test]
fn queued_transactions_take_their_snapshot_after_overlapping_predecessors_finish() {
    let mut manager = IndexedDbManager::new_in_memory();
    let first_db = open_database(&mut manager);
    let second_db = open_database(&mut manager);
    let first = queue(&manager, first_db, &["a"], TransactionMode::ReadWrite);
    let first_tx = manager.start_queued_transaction(first.handle()).unwrap();
    let reader = queue(&manager, second_db, &["a"], TransactionMode::ReadOnly);
    let last = queue(&manager, first_db, &["a"], TransactionMode::ReadWrite);
    assert!(!reader.handle().is_ready());
    assert!(manager.start_queued_transaction(reader.handle()).is_err());
    assert!(!last.handle().is_ready());
    manager.put(first_tx, "a", None, vec![1]).unwrap();
    manager.commit_transaction(first_tx).unwrap();
    drop(first);
    let reader_tx = manager.start_queued_transaction(reader.handle()).unwrap();
    assert_eq!(
        manager.get(reader_tx, "a", &Key::from(1)).unwrap(),
        RequestOutcome::Value(Some(vec![1].into()))
    );
    assert!(!last.handle().is_ready());
    manager.abort_transaction(reader_tx).unwrap();
    drop(reader);
    let last_tx = manager.start_queued_transaction(last.handle()).unwrap();
    assert_eq!(
        manager.put(last_tx, "a", None, vec![2]).unwrap(),
        Key::from(2)
    );
    manager.commit_transaction(last_tx).unwrap();
    drop(last);
}

#[test]
fn disjoint_writers_and_readers_preserve_other_commits_in_memory_and_on_disk() {
    let dir = TestDir::new();
    let mut manager = IndexedDbManager::new(&dir.path).unwrap();
    let database = open_database(&mut manager);
    let first = queue(&manager, database, &["a"], TransactionMode::ReadWrite);
    let second = queue(&manager, database, &["b"], TransactionMode::ReadWrite);
    let reader = queue(&manager, database, &["c"], TransactionMode::ReadOnly);
    let first_tx = manager.start_queued_transaction(first.handle()).unwrap();
    let second_tx = manager.start_queued_transaction(second.handle()).unwrap();
    let reader_tx = manager.start_queued_transaction(reader.handle()).unwrap();
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
    drop((first, second, reader));
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
            assert_eq!(
                manager.next_generated_key(second, "b").unwrap(),
                Key::from(1)
            );
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

#[test]
fn force_close_aborts_before_releasing_active_and_pending_admissions() {
    let mut manager = IndexedDbManager::new_in_memory();
    let first_db = open_database(&mut manager);
    let second_db = open_database(&mut manager);
    let first = queue(&manager, first_db, &["a"], TransactionMode::ReadWrite);
    let first_tx = manager.start_queued_transaction(first.handle()).unwrap();
    manager.put(first_tx, "a", None, vec![1]).unwrap();
    let pending = queue(&manager, first_db, &["a"], TransactionMode::ReadWrite);
    let reader = queue(&manager, second_db, &["a"], TransactionMode::ReadOnly);
    assert!(!reader.handle().is_ready());
    manager.force_close_database(first_db).unwrap();
    assert!(manager.commit_transaction(first_tx).is_err());
    assert!(!first.handle().is_ready());
    assert!(!pending.handle().is_ready());
    assert!(reader.handle().is_ready());
    let reader_tx = manager.start_queued_transaction(reader.handle()).unwrap();
    assert_eq!(
        manager.get(reader_tx, "a", &Key::from(1)).unwrap(),
        RequestOutcome::Value(None)
    );
    manager.commit_transaction(reader_tx).unwrap();
    drop((first, pending, reader));
}

#[test]
fn clearing_storage_retires_admissions_before_reopening_the_same_database() {
    for prefix in [false, true] {
        let mut manager = IndexedDbManager::new_in_memory();
        let db = open_database(&mut manager);
        let first = queue(&manager, db, &["a"], TransactionMode::ReadWrite);
        let tx = manager.start_queued_transaction(first.handle()).unwrap();
        manager.put(tx, "a", None, vec![1]).unwrap();
        let pending = queue(&manager, db, &["a"], TransactionMode::ReadOnly);
        if prefix {
            manager.clear_origins_with_prefix("orig").unwrap();
        } else {
            manager.clear_origin("origin").unwrap();
        }
        assert!(!first.handle().is_ready());
        assert!(!pending.handle().is_ready());
        let db = open_database(&mut manager);
        let read = queue(&manager, db, &["a"], TransactionMode::ReadOnly);
        let tx = manager.start_queued_transaction(read.handle()).unwrap();
        assert_eq!(
            manager.get(tx, "a", &Key::from(1)).unwrap(),
            RequestOutcome::Value(None)
        );
        manager.commit_transaction(tx).unwrap();
        drop((first, pending, read));
    }
}
