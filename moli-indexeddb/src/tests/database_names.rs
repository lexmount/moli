use super::*;
use std::sync::Arc;

const ORIGIN: &str = "https://database-names.test";

fn open(manager: &mut IndexedDbManager, name: &IndexedDbName, version: u64) -> OpenResult {
    manager
        .open(OpenOptions {
            origin: ORIGIN.into(),
            name: name.clone(),
            version: Some(version),
        })
        .unwrap()
}

#[test]
fn database_names_preserve_utf16_records_versions_and_legacy_persistence() {
    let dir = TestDir::new();
    let legacy = IndexedDbName::from("legacy-\0-数据库-💾");
    let mut manager = IndexedDbManager::new(&dir.path).unwrap();
    let db = open(&mut manager, &legacy, 1);
    manager
        .commit_transaction(db.upgrade_transaction.unwrap())
        .unwrap();
    manager.close_database(db.database).unwrap();
    drop(manager);
    let path = origin_path(&dir.path, ORIGIN);
    let persisted: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert!(persisted.get("databases_utf16").is_none());
    assert_eq!(persisted["databases"]["legacy-\0-数据库-💾"]["version"], 1);

    let names = [
        vec![],
        vec![0],
        vec![0xd800],
        vec![0xd801],
        vec![0xdfff],
        vec![0xfffd],
        vec![0xdfff, 0xd800],
        vec![0xd83d, 0xdcbe],
        vec![0xe000],
    ]
    .map(IndexedDbName::from_utf16);
    let mut manager = IndexedDbManager::new(&dir.path).unwrap();
    let db = open(&mut manager, &legacy, 1);
    assert_eq!(db.disposition, OpenDisposition::Existing);
    manager.close_database(db.database).unwrap();
    for (i, name) in names.iter().enumerate() {
        let db = open(&mut manager, name, 1);
        assert_eq!(
            db.disposition,
            OpenDisposition::UpgradeNeeded {
                old_version: 0,
                new_version: 1
            }
        );
        let tx = db.upgrade_transaction.unwrap();
        manager
            .create_object_store(tx, "s", ObjectStoreOptions::default())
            .unwrap();
        manager
            .put(tx, "s", Some(Key::from(1)), vec![i as u8])
            .unwrap();
        manager.commit_transaction(tx).unwrap();
        manager.close_database(db.database).unwrap();
    }
    drop(manager);
    let persisted: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(persisted["databases"].as_object().unwrap().len(), 6);
    assert_eq!(persisted["databases_utf16"].as_array().unwrap().len(), 4);

    let mut manager = IndexedDbManager::new(&dir.path).unwrap();
    let listed = manager
        .databases(ORIGIN)
        .unwrap()
        .into_iter()
        .map(|db| db.name)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(listed, names.iter().cloned().chain([legacy]).collect());
    for (i, name) in names.iter().enumerate() {
        let db = open(&mut manager, name, 1);
        assert_eq!(db.disposition, OpenDisposition::Existing);
        assert_eq!(manager.database_info(db.database).unwrap().name, *name);
        let tx = manager
            .begin_transaction(db.database, &["s".into()], TransactionMode::ReadOnly)
            .unwrap();
        assert_eq!(
            manager.get(tx, "s", &Key::from(1)).unwrap(),
            RequestOutcome::Value(Some(vec![i as u8].into()))
        );
        manager.commit_transaction(tx).unwrap();
        manager.close_database(db.database).unwrap();
    }

    let db = open(&mut manager, &names[2], 2);
    let tx = db.upgrade_transaction.unwrap();
    manager.put(tx, "s", Some(Key::from(1)), vec![99]).unwrap();
    manager.abort_transaction(tx).unwrap();
    let tx = manager
        .begin_transaction(db.database, &["s".into()], TransactionMode::ReadOnly)
        .unwrap();
    assert_eq!(
        manager.get(tx, "s", &Key::from(1)).unwrap(),
        RequestOutcome::Value(Some(vec![2].into()))
    );
    manager.commit_transaction(tx).unwrap();
    manager.close_database(db.database).unwrap();
    assert_eq!(
        manager.database_version(ORIGIN, &names[2]).unwrap(),
        Some(1)
    );
    let db = open(&mut manager, &names[3], 2);
    manager
        .commit_transaction(db.upgrade_transaction.unwrap())
        .unwrap();
    manager.close_database(db.database).unwrap();
    // A connection to another surrogate name must not block deleting this one.
    let held = open(&mut manager, &names[4], 1);
    manager.delete_database(ORIGIN, &names[2]).unwrap();
    assert!(matches!(
        manager.delete_database(ORIGIN, &names[4]),
        Err(IndexedDbError::InvalidState(_))
    ));
    manager.close_database(held.database).unwrap();
    drop(manager);
    let mut manager = IndexedDbManager::new(&dir.path).unwrap();
    for (i, name) in names.iter().enumerate() {
        assert_eq!(
            manager.database_version(ORIGIN, name).unwrap(),
            match i {
                2 => None,
                3 => Some(2),
                _ => Some(1),
            }
        );
    }
}

#[test]
fn connection_and_transaction_queues_use_exact_database_code_units() {
    let mut manager = IndexedDbManager::new_in_memory();
    let names = [0xd800, 0xdfff, 0xfffd].map(|unit| IndexedDbName::from_utf16(vec![unit]));
    let queues = manager.connection_request_queues();
    let leases = names
        .each_ref()
        .map(|name| queues.enqueue(ORIGIN, name, Arc::new(|| {})));
    let same = queues.enqueue(ORIGIN, &names[0], Arc::new(|| {}));
    assert!(leases.iter().all(|lease| lease.handle().is_head()));
    assert!(!same.handle().is_head());
    let databases = names.each_ref().map(|name| {
        let db = open(&mut manager, name, 1);
        let tx = db.upgrade_transaction.unwrap();
        manager
            .create_object_store(tx, "s", ObjectStoreOptions::default())
            .unwrap();
        manager.commit_transaction(tx).unwrap();
        db.database
    });
    let transactions = databases.map(|db| {
        manager
            .queue_transaction_start(
                db,
                &["s".into()],
                TransactionMode::ReadWrite,
                Arc::new(|| {}),
            )
            .unwrap()
    });
    let same_tx = manager
        .queue_transaction_start(
            databases[0],
            &["s".into()],
            TransactionMode::ReadWrite,
            Arc::new(|| {}),
        )
        .unwrap();
    assert!(transactions.iter().all(|lease| lease.handle().is_ready()));
    assert!(!same_tx.handle().is_ready());
    for (i, lease) in transactions.into_iter().enumerate() {
        let tx = manager.start_queued_transaction(lease.handle()).unwrap();
        manager
            .put(tx, "s", Some(Key::from(1)), vec![i as u8])
            .unwrap();
        manager.commit_transaction(tx).unwrap();
    }
    assert!(same_tx.handle().is_ready());
    let tx = manager.start_queued_transaction(same_tx.handle()).unwrap();
    assert_eq!(
        manager.get(tx, "s", &Key::from(1)).unwrap(),
        RequestOutcome::Value(Some(vec![0].into()))
    );
    manager.abort_transaction(tx).unwrap();
    drop(leases);
    assert!(same.handle().is_head());
}

#[test]
fn database_name_quota_accounting_preserves_other_databases_on_rollback() {
    let mut manager = IndexedDbManager::new_in_memory();
    let names = [0xd800, 0xdfff, 0xfffd].map(|unit| IndexedDbName::from_utf16(vec![unit]));
    for name in &names {
        let db = open(&mut manager, name, 1);
        manager
            .commit_transaction(db.upgrade_transaction.unwrap())
            .unwrap();
        manager.close_database(db.database).unwrap();
    }
    assert_eq!(manager.origin_usage_bytes(ORIGIN).unwrap(), 33);
    let db = open(&mut manager, &names[0], 2);
    let tx = db.upgrade_transaction.unwrap();
    manager
        .create_object_store(tx, "s", ObjectStoreOptions::default())
        .unwrap();
    assert!(matches!(
        manager.commit_transaction_with_quota(
            tx,
            IndexedDbQuotaCheck {
                quota: 33,
                non_indexed_db_usage: 0
            }
        ),
        Err(IndexedDbError::QuotaExceeded { .. })
    ));
    assert_eq!(manager.origin_usage_bytes(ORIGIN).unwrap(), 33);
    assert!(
        manager
            .databases(ORIGIN)
            .unwrap()
            .iter()
            .all(|db| db.version == 1)
    );
}
