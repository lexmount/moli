use super::*;

fn seeded() -> Database {
    let mut database = database();
    database
        .begin(TX, TransactionMode::VersionChange, BTreeSet::new(), Some(2))
        .unwrap();
    database
        .create_index(
            TX,
            "a",
            "by-tag",
            IndexOptions {
                key_path: KeyPath::String("tag".into()),
                unique: true,
                multi_entry: false,
            },
        )
        .unwrap();
    database.commit(TX).unwrap();
    begin(&mut database, TransactionMode::ReadWrite);
    put(
        &mut database,
        None,
        b"kept",
        vec![IndexKeys {
            name: "by-tag".into(),
            keys: vec!["alpha".into()],
        }],
        None,
    )
    .unwrap();
    database.commit(TX).unwrap();
    database
}

fn upgrade(database: &mut Database) {
    database
        .begin(TX, TransactionMode::VersionChange, BTreeSet::new(), Some(3))
        .unwrap();
}

#[test]
fn schema_rename_keeps_ids_records_index_entries_and_key_generator() {
    let mut database = seeded();
    let before = database.committed().clone();
    let store_id = before.stores["a"].id;
    let index_id = before.stores["a"].indexes["by-tag"].id;
    upgrade(&mut database);
    database.rename_object_store(TX, "a", "renamed").unwrap();
    database
        .rename_index(TX, "renamed", "by-tag", "new-tag")
        .unwrap();
    assert_eq!(
        database.committed(),
        &before,
        "pending names are not published"
    );
    let candidate = database.candidate_metadata(TX).unwrap();
    assert!(!candidate.stores.contains_key("a"));
    assert_eq!(candidate.stores["renamed"].id, store_id);
    assert_eq!(candidate.stores["renamed"].indexes["new-tag"].id, index_id);
    assert_eq!(candidate.usage, before.usage + 7);
    let records = database
        .query(
            TX,
            "renamed",
            &Query {
                source: QuerySource::Index("new-tag".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].key, "alpha".into());
    assert_eq!(records[0].primary_key, 1.into());
    assert_eq!(records[0].value.as_ref().unwrap().wire_bytes(), b"kept");
    assert_eq!(
        database
            .write(
                TX,
                "renamed",
                RecordWrite {
                    key: None,
                    value: b"next".to_vec().into(),
                    indexes: vec![IndexKeys {
                        name: "new-tag".into(),
                        keys: vec!["beta".into()],
                    }],
                    add_only: false,
                    quota: None,
                }
            )
            .unwrap(),
        2.into()
    );
    database.commit(TX).unwrap();
    assert_eq!(database.committed().stores["renamed"].id, store_id);
    assert_eq!(
        database.committed().stores["renamed"].indexes["new-tag"].id,
        index_id
    );
}

#[test]
fn schema_rename_constraint_errors_leave_the_upgrade_usable() {
    let mut database = seeded();
    let before = database.committed().clone();
    let store_id = before.stores["a"].id;
    let index_id = before.stores["a"].indexes["by-tag"].id;
    upgrade(&mut database);
    let sibling = database
        .create_index(
            TX,
            "a",
            "other",
            IndexOptions {
                key_path: KeyPath::String("other".into()),
                unique: false,
                multi_entry: false,
            },
        )
        .unwrap();
    assert!(matches!(
        database.rename_object_store(TX, "a", "b"),
        Err(IndexedDbError::Constraint(_))
    ));
    assert!(matches!(
        database.rename_index(TX, "a", "by-tag", "other"),
        Err(IndexedDbError::Constraint(_))
    ));
    assert!(matches!(
        database.rename_object_store(TX, "missing", "new"),
        Err(IndexedDbError::NotFound(_))
    ));
    assert!(matches!(
        database.rename_index(TX, "a", "missing", "new"),
        Err(IndexedDbError::NotFound(_))
    ));
    let metadata = database.candidate_metadata(TX).unwrap();
    assert_eq!(metadata.stores["a"].id, store_id);
    assert_eq!(metadata.stores["a"].indexes["by-tag"].id, index_id);
    assert_eq!(metadata.stores["a"].indexes["other"].id, sibling.id);
    // Empty names are valid, and a same-name assignment leaves accounting unchanged.
    database.rename_object_store(TX, "a", "").unwrap();
    database.rename_index(TX, "", "by-tag", "").unwrap();
    let usage = database.candidate_metadata(TX).unwrap().usage;
    database.rename_object_store(TX, "", "").unwrap();
    database.rename_index(TX, "", "", "").unwrap();
    assert_eq!(database.candidate_metadata(TX).unwrap().usage, usage);
    database.commit(TX).unwrap();
    assert_eq!(database.committed().stores[""].id, store_id);
    assert_eq!(database.committed().stores[""].indexes[""].id, index_id);
}

#[test]
fn schema_rename_abort_and_commit_quota_failure_restore_the_original_names() {
    let mut database = seeded();
    let before = database.committed().clone();
    for quota_failure in [false, true] {
        upgrade(&mut database);
        let new_name = "long".repeat(64);
        database.rename_object_store(TX, "a", &new_name).unwrap();
        database
            .rename_index(TX, &new_name, "by-tag", "renamed-index")
            .unwrap();
        if quota_failure {
            assert!(matches!(
                database.commit_with_quota(
                    TX,
                    Some(QuotaCheck {
                        quota: before.usage,
                        other_usage: 0,
                    })
                ),
                Err(IndexedDbError::QuotaExceeded { .. })
            ));
        } else {
            database.abort(TX).unwrap();
        }
        assert_eq!(database.committed(), &before);
        assert!(database.connection.is_autocommit());
        assert!(!database.is_transaction_active());
        begin(&mut database, TransactionMode::ReadOnly);
        assert_eq!(
            database
                .get(TX, "a", &1.into())
                .unwrap()
                .unwrap()
                .wire_bytes(),
            b"kept"
        );
        database.commit(TX).unwrap();
    }
}

#[test]
fn schema_rename_and_reusing_the_old_name_preserve_separate_storage_identities() {
    let mut database = seeded();
    let original = database.committed().stores["a"].id;
    upgrade(&mut database);
    database.rename_object_store(TX, "a", "kept").unwrap();
    database
        .create_object_store(TX, "a", ObjectStoreOptions::default())
        .unwrap();
    assert_ne!(
        database.candidate_metadata(TX).unwrap().stores["a"].id,
        original
    );
    assert_eq!(database.count(TX, "a").unwrap(), 0);
    assert_eq!(database.count(TX, "kept").unwrap(), 1);
    database.delete_object_store(TX, "a").unwrap();
    assert_eq!(
        database
            .get(TX, "kept", &1.into())
            .unwrap()
            .unwrap()
            .wire_bytes(),
        b"kept"
    );
    database.commit(TX).unwrap();
    assert_eq!(database.committed().stores["kept"].id, original);
}

#[test]
fn schema_rename_requires_an_active_versionchange_transaction() {
    let mut database = seeded();
    for mode in [TransactionMode::ReadOnly, TransactionMode::ReadWrite] {
        begin(&mut database, mode);
        assert!(matches!(
            database.rename_object_store(TX, "a", "renamed"),
            Err(IndexedDbError::InvalidState(_))
        ));
        assert!(matches!(
            database.rename_index(TX, "a", "by-tag", "renamed"),
            Err(IndexedDbError::InvalidState(_))
        ));
        database.abort(TX).unwrap();
    }
    assert!(matches!(
        database.rename_object_store(TX, "a", "renamed"),
        Err(IndexedDbError::TransactionInactive(_))
    ));
    assert!(matches!(
        database.rename_index(TX, "a", "by-tag", "renamed"),
        Err(IndexedDbError::TransactionInactive(_))
    ));
}

#[test]
fn renaming_an_index_and_reusing_its_old_name_keeps_entries_separate() {
    let mut database = seeded();
    let original = database.committed().stores["a"].indexes["by-tag"].id;
    upgrade(&mut database);
    database.rename_index(TX, "a", "by-tag", "kept").unwrap();
    let replacement = database
        .create_index(
            TX,
            "a",
            "by-tag",
            IndexOptions {
                key_path: KeyPath::String("other".into()),
                unique: false,
                multi_entry: false,
            },
        )
        .unwrap();
    assert_ne!(original, replacement.id);
    let old_entries = database
        .query(
            TX,
            "a",
            &Query {
                source: QuerySource::Index("kept".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(old_entries.len(), 1);
    assert!(
        database
            .query(
                TX,
                "a",
                &Query {
                    source: QuerySource::Index("by-tag".into()),
                    ..Default::default()
                }
            )
            .unwrap()
            .is_empty()
    );
    database.delete_index(TX, "a", "by-tag").unwrap();
    assert_eq!(
        database
            .query(
                TX,
                "a",
                &Query {
                    source: QuerySource::Index("kept".into()),
                    ..Default::default()
                }
            )
            .unwrap(),
        old_entries
    );
    database.commit(TX).unwrap();
    assert_eq!(
        database.committed().stores["a"].indexes["kept"].id,
        original
    );
}
