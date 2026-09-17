use super::*;

#[test]
fn cursor_scan_revisions_follow_successful_writes_and_survive_rejected_writes() {
    let mut manager = IndexedDbManager::new_in_memory();
    let open = manager
        .open(OpenOptions {
            origin: "origin".into(),
            name: "revisions".into(),
            version: Some(1),
        })
        .unwrap();
    let tx = open.upgrade_transaction.unwrap();
    manager
        .create_object_store(
            tx,
            "s",
            ObjectStoreOptions {
                auto_increment: true,
                ..Default::default()
            },
        )
        .unwrap();
    manager
        .create_object_store(tx, "other", ObjectStoreOptions::default())
        .unwrap();
    let initial = manager.transaction_record_revision(tx).unwrap();
    let key = manager.add(tx, "s", None, vec![1]).unwrap();
    let written = manager.transaction_record_revision(tx).unwrap();
    assert_ne!(initial, written);
    manager.entries(tx, "s").unwrap();
    manager.generate_key(tx, "s").unwrap();
    manager
        .delete(tx, "s", &Key::number(100.0).unwrap())
        .unwrap();
    manager.clear(tx, "other").unwrap();
    assert_eq!(manager.transaction_record_revision(tx).unwrap(), written);
    assert!(matches!(
        manager.add(tx, "s", Some(key.clone()), vec![2]),
        Err(IndexedDbError::Constraint(_))
    ));
    assert!(matches!(
        manager.put_with_quota(
            tx,
            "s",
            Some(key.clone()),
            vec![3],
            IndexedDbQuotaCheck {
                quota: 0,
                non_indexed_db_usage: 0
            }
        ),
        Err(IndexedDbError::QuotaExceeded { .. })
    ));
    assert_eq!(manager.transaction_record_revision(tx).unwrap(), written);
    assert_eq!(
        manager.get(tx, "s", &key).unwrap(),
        RequestOutcome::Value(Some(vec![1].into()))
    );
    manager.put(tx, "s", Some(key.clone()), vec![4]).unwrap();
    let replaced = manager.transaction_record_revision(tx).unwrap();
    assert_ne!(written, replaced);
    manager.delete(tx, "s", &key).unwrap();
    let deleted = manager.transaction_record_revision(tx).unwrap();
    assert_ne!(replaced, deleted);
    manager.put(tx, "s", Some(key), vec![5]).unwrap();
    let restored = manager.transaction_record_revision(tx).unwrap();
    manager.clear(tx, "s").unwrap();
    assert_ne!(manager.transaction_record_revision(tx).unwrap(), restored);
    assert!(manager.entries(tx, "s").unwrap().is_empty());
    manager.commit_transaction(tx).unwrap();
    assert!(manager.transaction_record_revision(tx).is_err());
    let tx = manager
        .begin_transaction(open.database, &["s".into()], TransactionMode::ReadOnly)
        .unwrap();
    let initial = manager.transaction_record_revision(tx).unwrap();
    assert!(manager.put(tx, "s", Key::number(1.0), vec![6]).is_err());
    assert_eq!(manager.transaction_record_revision(tx).unwrap(), initial);
    manager.abort_transaction(tx).unwrap();
    assert!(manager.transaction_record_revision(tx).is_err());
}
