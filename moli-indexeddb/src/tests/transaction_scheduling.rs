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
fn closing_connections_block_deletion_until_their_backend_transactions_finish() {
    for commit in [false, true] {
        let mut manager = IndexedDbManager::new_in_memory();
        let db = open_database(&mut manager);
        let tx = manager
            .begin_transaction(db, &["a".into()], TransactionMode::ReadWrite)
            .unwrap();
        manager.put(tx, "a", None, vec![1]).unwrap();
        manager.close_database(db).unwrap();
        assert!(matches!(
            manager.delete_database("origin", "db"),
            Err(IndexedDbError::InvalidState(_))
        ));
        assert_eq!(manager.database_version("origin", "db").unwrap(), Some(1));
        if commit {
            manager.commit_transaction(tx).unwrap();
        } else {
            manager.abort_transaction(tx).unwrap();
        }
        let reader_db = open_database(&mut manager);
        let reader = manager
            .begin_transaction(reader_db, &["a".into()], TransactionMode::ReadOnly)
            .unwrap();
        assert_eq!(
            manager.get(reader, "a", &Key::from(1)).unwrap(),
            RequestOutcome::Value(commit.then(|| vec![1].into()))
        );
        manager.commit_transaction(reader).unwrap();
        manager.close_database(reader_db).unwrap();
        manager.delete_database("origin", "db").unwrap();
        assert_eq!(manager.database_version("origin", "db").unwrap(), None);
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
