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

#[test]
fn admission_starts_once_and_backend_retains_a_started_lease_until_completion() {
    use crate::TransactionRequestStatus;
    let mut manager = IndexedDbManager::new_in_memory();
    let db = open_database(&mut manager);
    let reader = queue(&manager, db, &["a"], TransactionMode::ReadOnly);
    let retained = reader.handle().clone();
    let tx = manager.start_queued_transaction(&retained).unwrap();
    assert_eq!(retained.status(), TransactionRequestStatus::Started);
    assert!(manager.start_queued_transaction(&retained).is_err());
    let writer = queue(&manager, db, &["a"], TransactionMode::ReadWrite);
    drop(reader);
    assert_eq!(retained.status(), TransactionRequestStatus::Started);
    assert!(!writer.handle().is_ready());
    manager.commit_transaction(tx).unwrap();
    assert_eq!(retained.status(), TransactionRequestStatus::Finished);
    assert!(writer.handle().is_ready());
    let tx = manager.start_queued_transaction(writer.handle()).unwrap();
    manager.abort_transaction(tx).unwrap();
    assert_eq!(writer.handle().status(), TransactionRequestStatus::Finished);
}

#[test]
fn closing_connections_drain_already_accepted_transactions_and_block_new_admissions() {
    use crate::TransactionRequestStatus;
    let mut manager = IndexedDbManager::new_in_memory();
    let first_db = open_database(&mut manager);
    let closing_db = open_database(&mut manager);
    let first = queue(&manager, first_db, &["a"], TransactionMode::ReadWrite);
    let first_tx = manager.start_queued_transaction(first.handle()).unwrap();
    manager.put(first_tx, "a", None, vec![7]).unwrap();
    let accepted = queue(&manager, closing_db, &["a"], TransactionMode::ReadOnly);
    assert_eq!(
        accepted.handle().status(),
        TransactionRequestStatus::Pending
    );
    manager.close_database(closing_db).unwrap();
    assert_eq!(
        accepted.handle().status(),
        TransactionRequestStatus::Pending
    );
    assert!(
        manager
            .queue_transaction_start(
                closing_db,
                &["a".into()],
                TransactionMode::ReadOnly,
                Arc::new(|| {})
            )
            .is_err()
    );
    assert!(
        manager
            .begin_transaction(closing_db, &["a".into()], TransactionMode::ReadOnly)
            .is_err()
    );
    manager.commit_transaction(first_tx).unwrap();
    manager.close_database(first_db).unwrap();
    assert_eq!(accepted.handle().status(), TransactionRequestStatus::Ready);
    assert!(manager.delete_database("origin", "db").is_err());
    let tx = manager.start_queued_transaction(accepted.handle()).unwrap();
    assert_eq!(
        manager.get(tx, "a", &Key::from(1)).unwrap(),
        RequestOutcome::Value(Some(vec![7].into()))
    );
    manager.commit_transaction(tx).unwrap();
    manager.delete_database("origin", "db").unwrap();
}

#[test]
fn dropping_unstarted_admission_finishes_a_closing_connection() {
    let mut manager = IndexedDbManager::new_in_memory();
    let db = open_database(&mut manager);
    let accepted = queue(&manager, db, &["a"], TransactionMode::ReadOnly);
    manager.close_database(db).unwrap();
    assert!(manager.delete_database("origin", "db").is_err());
    drop(accepted);
    manager.delete_database("origin", "db").unwrap();
}

#[test]
fn storage_clear_notifies_canceled_active_pending_and_idle_owners_once() {
    use crate::TransactionRequestStatus;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let mut manager = IndexedDbManager::new_in_memory();
    let active_db = open_database(&mut manager);
    let idle_db = open_database(&mut manager);
    let close_wakes = Arc::new(AtomicUsize::new(0));
    let mut signals = Vec::new();
    for db in [active_db, idle_db] {
        let wakes = close_wakes.clone();
        signals.push(manager.observe_database_close(
            db,
            Arc::new(move || {
                wakes.fetch_add(1, Ordering::SeqCst);
            }),
        ));
    }
    let admission_wakes = Arc::new(AtomicUsize::new(0));
    let wake: crate::TransactionRequestWake = {
        let wakes = admission_wakes.clone();
        Arc::new(move || {
            wakes.fetch_add(1, Ordering::SeqCst);
        })
    };
    let first = manager
        .queue_transaction_start(
            active_db,
            &["a".into()],
            TransactionMode::ReadWrite,
            wake.clone(),
        )
        .unwrap();
    let tx = manager.start_queued_transaction(first.handle()).unwrap();
    let pending = manager
        .queue_transaction_start(active_db, &["a".into()], TransactionMode::ReadOnly, wake)
        .unwrap();
    manager.clear_origin("origin").unwrap();
    assert_eq!(first.handle().status(), TransactionRequestStatus::Canceled);
    assert_eq!(
        pending.handle().status(),
        TransactionRequestStatus::Canceled
    );
    assert_eq!(admission_wakes.load(Ordering::SeqCst), 2);
    assert_eq!(close_wakes.load(Ordering::SeqCst), 2);
    assert!(
        signals
            .iter()
            .all(crate::DatabaseCloseSignal::is_forced_closed)
    );
    assert!(manager.commit_transaction(tx).is_err());
    manager.clear_origin("origin").unwrap();
    assert_eq!(admission_wakes.load(Ordering::SeqCst), 2);
    assert_eq!(close_wakes.load(Ordering::SeqCst), 2);
    let db = open_database(&mut manager);
    let wakes = close_wakes.clone();
    let normal_close = manager.observe_database_close(
        db,
        Arc::new(move || {
            wakes.fetch_add(1, Ordering::SeqCst);
        }),
    );
    manager.close_database(db).unwrap();
    assert!(!normal_close.is_forced_closed());
    assert_eq!(close_wakes.load(Ordering::SeqCst), 2);
}

#[test]
fn retiring_connections_wakes_only_survivors_after_rolling_back_the_whole_owner() {
    use crate::TransactionRequestStatus;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let mut manager = IndexedDbManager::new_in_memory();
    let active_db = open_database(&mut manager);
    let pending_db = open_database(&mut manager);
    let surviving_db = open_database(&mut manager);
    let retired_wakes = Arc::new(AtomicUsize::new(0));
    let retired_wake: crate::TransactionRequestWake = {
        let wakes = retired_wakes.clone();
        Arc::new(move || {
            wakes.fetch_add(1, Ordering::SeqCst);
        })
    };
    let mut signals = Vec::new();
    for database in [active_db, pending_db] {
        signals.push(manager.observe_database_close(database, retired_wake.clone()));
    }
    let active = manager
        .queue_transaction_start(
            active_db,
            &["a".into()],
            TransactionMode::ReadWrite,
            retired_wake.clone(),
        )
        .unwrap();
    let backend = manager.start_queued_transaction(active.handle()).unwrap();
    manager.put(backend, "a", None, vec![1]).unwrap();
    let pending = manager
        .queue_transaction_start(
            pending_db,
            &["a".into()],
            TransactionMode::ReadWrite,
            retired_wake,
        )
        .unwrap();
    let surviving_wakes = Arc::new(AtomicUsize::new(0));
    let wake = surviving_wakes.clone();
    let surviving = manager
        .queue_transaction_start(
            surviving_db,
            &["a".into()],
            TransactionMode::ReadOnly,
            Arc::new(move || {
                wake.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .unwrap();
    assert_eq!(
        manager.retire_database_connections([active_db, pending_db]),
        2
    );
    assert_eq!(retired_wakes.load(Ordering::SeqCst), 0);
    assert_eq!(surviving_wakes.load(Ordering::SeqCst), 1);
    assert_eq!(active.handle().status(), TransactionRequestStatus::Canceled);
    assert_eq!(
        pending.handle().status(),
        TransactionRequestStatus::Canceled
    );
    assert!(signals.iter().all(|signal| !signal.is_forced_closed()));
    assert!(manager.commit_transaction(backend).is_err());
    assert!(surviving.handle().is_ready());
    let reader = manager
        .start_queued_transaction(surviving.handle())
        .unwrap();
    assert_eq!(
        manager.get(reader, "a", &Key::from(1)).unwrap(),
        RequestOutcome::Value(None)
    );
    manager.commit_transaction(reader).unwrap();
    assert_eq!(retired_wakes.load(Ordering::SeqCst), 0);
}

#[test]
fn commit_quota_failure_releases_started_admission_after_rollback() {
    use crate::TransactionRequestStatus;
    let mut manager = IndexedDbManager::new_in_memory();
    let db = open_database(&mut manager);
    let initial_usage = manager.origin_usage_bytes("origin").unwrap();
    let first = queue(&manager, db, &["a"], TransactionMode::ReadWrite);
    let second = queue(&manager, db, &["b"], TransactionMode::ReadWrite);
    let first_tx = manager.start_queued_transaction(first.handle()).unwrap();
    let second_tx = manager.start_queued_transaction(second.handle()).unwrap();
    let successor = queue(&manager, db, &["b"], TransactionMode::ReadOnly);
    let quota = IndexedDbQuotaCheck {
        quota: initial_usage + 180,
        non_indexed_db_usage: 0,
    };
    manager
        .put_with_quota(first_tx, "a", None, vec![1; 100], quota)
        .unwrap();
    manager
        .put_with_quota(second_tx, "b", None, vec![2; 100], quota)
        .unwrap();
    manager
        .commit_transaction_with_quota(first_tx, quota)
        .unwrap();
    assert!(matches!(
        manager.commit_transaction_with_quota(second_tx, quota),
        Err(IndexedDbError::QuotaExceeded { .. })
    ));
    assert_eq!(second.handle().status(), TransactionRequestStatus::Finished);
    assert!(successor.handle().is_ready());
    let read = manager
        .start_queued_transaction(successor.handle())
        .unwrap();
    assert_eq!(
        manager.get(read, "b", &Key::from(1)).unwrap(),
        RequestOutcome::Value(None)
    );
    manager.commit_transaction(read).unwrap();
}
