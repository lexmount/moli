mod database_names;
mod schema_names;
mod schema_rename;
mod storage_contracts;
use super::*;
use crate::{IndexedDbExternalObject as External, IndexedDbValue, KeyPath};

pub(super) const TX: TransactionHandle = TransactionHandle::from_raw(1);

fn identity() -> DatabaseIdentity {
    DatabaseIdentity {
        owner_key: b"partition/default/storage-key/top-site/bucket-17".to_vec(),
        name: "database".into(),
        generation: [42; 16],
    }
}

fn empty() -> Database {
    Database::create(
        moli_sqlite::open_private_memory(Default::default()).unwrap(),
        identity(),
    )
    .unwrap()
}

#[test]
fn database_versions_preserve_the_webidl_range_and_rollback() {
    let mut database = database();
    for version in [1_u64 << 31, 1_u64 << 32, (1_u64 << 53) - 1] {
        let committed = database.committed().clone();
        database
            .begin(
                TX,
                TransactionMode::VersionChange,
                BTreeSet::new(),
                Some(version),
            )
            .unwrap();
        assert_eq!(database.candidate_metadata(TX).unwrap().version, version);
        assert_eq!(database.committed(), &committed);
        database.abort(TX).unwrap();
        assert_eq!(database.committed(), &committed);

        database
            .begin(
                TX,
                TransactionMode::VersionChange,
                BTreeSet::new(),
                Some(version),
            )
            .unwrap();
        assert_eq!(database.commit(TX).unwrap().version, version);
        assert_eq!(database.committed().version, version);
        for rejected in [version - 1, version] {
            assert!(matches!(
                database.begin(
                    TX,
                    TransactionMode::VersionChange,
                    BTreeSet::new(),
                    Some(rejected)
                ),
                Err(IndexedDbError::Version(_))
            ));
            assert!(database.connection.is_autocommit());
        }

        // Ordinary writes take the committed-version equality path, which
        // must preserve the same unsigned identity as upgrades and reopen.
        begin(&mut database, TransactionMode::ReadWrite);
        put(
            &mut database,
            Some(1.into()),
            &version.to_be_bytes(),
            vec![],
            None,
        )
        .unwrap();
        assert_eq!(database.candidate_metadata(TX).unwrap().version, version);
        database.commit(TX).unwrap();
        begin(&mut database, TransactionMode::ReadOnly);
        assert_eq!(
            database
                .get(TX, "a", &1.into())
                .unwrap()
                .unwrap()
                .wire_bytes(),
            &version.to_be_bytes()
        );
        database.commit(TX).unwrap();
    }
}

#[test]
fn commit_quota_failure_rolls_back_records_generator_and_usage() {
    let mut database = database();
    let original = database.committed().clone();
    for other_usage in [4096, u64::MAX] {
        begin(&mut database, TransactionMode::ReadWrite);
        assert_eq!(
            put(&mut database, None, b"pending", vec![], None).unwrap(),
            1.into()
        );
        let candidate = database.candidate_metadata(TX).unwrap().usage;
        let quota = if other_usage == u64::MAX {
            u64::MAX
        } else {
            other_usage + candidate - 1
        };
        assert!(matches!(
            database.commit_with_quota(TX, Some(QuotaCheck { quota, other_usage })),
            Err(IndexedDbError::QuotaExceeded { .. })
        ));
        assert_eq!(database.committed(), &original);
        assert!(!database.is_transaction_active());
        begin(&mut database, TransactionMode::ReadOnly);
        assert_eq!(database.count(TX, "a").unwrap(), 0);
        database.commit(TX).unwrap();
    }
    begin(&mut database, TransactionMode::ReadWrite);
    assert_eq!(
        put(&mut database, None, b"kept", vec![], None).unwrap(),
        1.into()
    );
    let candidate = database.candidate_metadata(TX).unwrap().usage;
    database
        .commit_with_quota(
            TX,
            Some(QuotaCheck {
                quota: 4096 + candidate,
                other_usage: 4096,
            }),
        )
        .unwrap();
    assert_eq!(database.committed().usage, candidate);
}

pub(super) fn database() -> Database {
    let mut database = empty();
    database
        .begin(TX, TransactionMode::VersionChange, BTreeSet::new(), Some(1))
        .unwrap();
    for name in ["a", "b"] {
        database
            .create_object_store(
                TX,
                name,
                ObjectStoreOptions {
                    auto_increment: true,
                    ..Default::default()
                },
            )
            .unwrap();
    }
    database.commit(TX).unwrap();
    database
}

#[test]
fn commit_metadata_failure_rolls_back_and_allows_the_next_transaction() {
    let mut database = database();
    database
        .begin(TX, TransactionMode::VersionChange, BTreeSet::new(), Some(2))
        .unwrap();
    let committed = database.committed().clone();
    database
        .connection
        .execute_batch("DROP TABLE indexes")
        .unwrap();
    assert!(database.commit(TX).is_err());
    assert!(database.connection.is_autocommit());
    assert!(!database.is_transaction_active());
    assert_eq!(database.committed(), &committed);
    // The injected DROP was transactional too; a later upgrade can read the
    // intact schema and publish normally instead of remaining stuck in BEGIN.
    database
        .begin(TX, TransactionMode::VersionChange, BTreeSet::new(), Some(2))
        .unwrap();
    database.commit(TX).unwrap();
    assert_eq!(database.committed().version, 2);
}

pub(super) fn begin(database: &mut Database, mode: TransactionMode) {
    database
        .begin(TX, mode, BTreeSet::from(["a".into()]), None)
        .unwrap();
}

pub(super) fn put(
    database: &mut Database,
    key: Option<Key>,
    bytes: &[u8],
    indexes: Vec<IndexKeys>,
    quota: Option<u64>,
) -> Result<Key, IndexedDbError> {
    database.write(
        TX,
        "a",
        RecordWrite {
            key,
            value: bytes.to_vec().into(),
            indexes,
            add_only: false,
            quota,
        },
    )
}

fn index_keys(key: &str) -> Vec<IndexKeys> {
    vec![IndexKeys {
        name: "unique".into(),
        keys: vec![key.into()],
    }]
}

fn add_unique_index(database: &mut Database) {
    database
        .begin(TX, TransactionMode::VersionChange, BTreeSet::new(), Some(2))
        .unwrap();
    database
        .create_index(
            TX,
            "a",
            "unique",
            IndexOptions {
                key_path: KeyPath::from("value"),
                unique: true,
                multi_entry: false,
            },
        )
        .unwrap();
    database.commit(TX).unwrap();
}

#[test]
fn committed_schema_and_usage_publish_only_after_commit_while_readers_overlap() {
    let mut database = empty();
    assert_eq!(database.committed().version, 0);
    assert_eq!(database.committed().usage, 0);
    database
        .begin(TX, TransactionMode::VersionChange, BTreeSet::new(), Some(1))
        .unwrap();
    database
        .create_object_store(TX, "a", ObjectStoreOptions::default())
        .unwrap();
    assert_eq!(database.committed().version, 0);
    assert!(database.committed().stores.is_empty());
    assert_eq!(
        database.candidate_metadata(TX).unwrap().usage,
        8 + 8 + 1 + 9
    );
    database.commit(TX).unwrap();
    let committed = database.committed().clone();
    assert_eq!(committed.version, 1);
    begin(&mut database, TransactionMode::ReadOnly);
    let second = TransactionHandle::from_raw(2);
    database
        .begin(
            second,
            TransactionMode::ReadOnly,
            BTreeSet::from(["a".into()]),
            None,
        )
        .unwrap();
    assert!(
        database
            .begin(
                TransactionHandle::from_raw(3),
                TransactionMode::ReadWrite,
                BTreeSet::from(["a".into()]),
                None
            )
            .is_err()
    );
    database.commit(second).unwrap();
    assert!(matches!(
        database.prepare_write(TX, "a", Some(1.into()), false),
        Err(IndexedDbError::ReadOnly(_))
    ));
    database.commit(TX).unwrap();
    assert_eq!(database.committed(), &committed);
}

#[test]
fn readonly_scope_and_transaction_identity_are_enforced_before_sql() {
    let mut database = database();
    begin(&mut database, TransactionMode::ReadOnly);
    assert!(matches!(
        database.count(TX, "b"),
        Err(IndexedDbError::InvalidState(_))
    ));
    assert!(matches!(
        database.count(TransactionHandle::from_raw(99), "a"),
        Err(IndexedDbError::TransactionInactive(_))
    ));
    assert!(matches!(
        database.clear(TX, "a"),
        Err(IndexedDbError::ReadOnly(_))
    ));
    database.abort(TX).unwrap();
}

#[test]
fn quota_failure_rolls_back_generated_key_value_and_usage_but_allows_next_request() {
    let mut database = database();
    let initial = database.committed().usage;
    begin(&mut database, TransactionMode::ReadWrite);
    assert!(matches!(
        put(&mut database, None, &[1; 100], vec![], Some(initial)),
        Err(IndexedDbError::QuotaExceeded { .. })
    ));
    assert!(database.is_transaction_active());
    assert_eq!(database.count(TX, "a").unwrap(), 0);
    assert_eq!(database.candidate_metadata(TX).unwrap().usage, initial);
    assert_eq!(
        put(&mut database, None, b"saved", vec![], None).unwrap(),
        Key::from(1)
    );
    assert_eq!(database.committed().usage, initial);
    database.commit(TX).unwrap();
    assert_eq!(database.committed().usage, initial + 8 + 5);
}

#[test]
fn unique_conflict_restores_old_external_objects_indexes_and_id_counters() {
    let mut database = database();
    add_unique_index(&mut database);
    begin(&mut database, TransactionMode::ReadWrite);
    let first = database.prepare_write(TX, "a", None, false).unwrap();
    let value = IndexedDbValue::new(
        vec![11],
        vec![External::Blob {
            bytes: vec![7; CHUNK_BYTES + 17],
            mime_type: "a/b".into(),
        }],
    );
    database
        .finish_write(TX, first.id, value.clone(), index_keys("first"), None)
        .unwrap();
    put(&mut database, None, b"second", index_keys("second"), None).unwrap();
    let usage = database.candidate_metadata(TX).unwrap().usage;
    let next_object: i64 = database
        .connection
        .query_row("SELECT next_object_id FROM database_metadata", [], |row| {
            row.get(0)
        })
        .unwrap();
    let replacement = database
        .prepare_write(TX, "a", Some(1.into()), false)
        .unwrap();
    let other = IndexedDbValue::new(
        vec![9],
        vec![External::File {
            bytes: vec![8; CHUNK_BYTES * 2 + 1],
            mime_type: "c/d".into(),
            name: "replacement".into(),
            last_modified: 3.5,
        }],
    );
    assert!(matches!(
        database.finish_write(TX, replacement.id, other, index_keys("second"), None),
        Err(IndexedDbError::Constraint(_))
    ));
    assert_eq!(database.get(TX, "a", &1.into()).unwrap(), Some(value));
    assert_eq!(database.candidate_metadata(TX).unwrap().usage, usage);
    assert_eq!(
        database
            .connection
            .query_row("SELECT next_object_id FROM database_metadata", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        next_object
    );
    assert_eq!(
        database
            .connection
            .query_row("SELECT count(*) FROM index_entries", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        put(&mut database, None, b"third", index_keys("third"), None).unwrap(),
        Key::from(3)
    );
    database.commit(TX).unwrap();
}

#[test]
fn failed_generated_unique_write_and_owner_cancellation_reuse_the_key() {
    let mut database = database();
    add_unique_index(&mut database);
    begin(&mut database, TransactionMode::ReadWrite);
    put(&mut database, None, b"one", index_keys("same"), None).unwrap();
    assert!(matches!(
        put(&mut database, None, b"bad", index_keys("same"), None),
        Err(IndexedDbError::Constraint(_))
    ));
    let canceled = database.prepare_write(TX, "a", None, false).unwrap();
    assert_eq!(canceled.key, Key::from(2));
    assert!(database.commit(TX).is_err());
    assert!(database.count(TX, "a").is_err());
    database.cancel_write(TX, canceled.id).unwrap();
    let current = database.prepare_write(TX, "a", None, false).unwrap();
    assert_eq!(current.key, Key::from(2));
    assert!(matches!(
        database.finish_write(TX, canceled.id, vec![0].into(), index_keys("new"), None),
        Err(IndexedDbError::InvalidState(_))
    ));
    database
        .finish_write(TX, current.id, vec![2].into(), index_keys("new"), None)
        .unwrap();
    database.commit(TX).unwrap();
}

#[test]
fn explicit_key_advancement_and_generator_boundary_rollback_are_atomic() {
    let mut database = database();
    begin(&mut database, TransactionMode::ReadWrite);
    let usage = database.committed().usage;
    assert!(
        put(
            &mut database,
            Some(999.into()),
            b"too large",
            vec![],
            Some(usage)
        )
        .is_err()
    );
    assert_eq!(
        put(&mut database, None, b"one", vec![], None).unwrap(),
        Key::from(1)
    );
    assert_eq!(
        database
            .connection
            .execute(
                "UPDATE object_stores SET generator=?1 WHERE name=?2",
                rusqlite::params![
                    (MAX_AUTO_INCREMENT_KEY - 1) as i64,
                    names::encode(&"a".into()).unwrap()
                ],
            )
            .unwrap(),
        1
    );
    let request = database.prepare_write(TX, "a", None, false).unwrap();
    assert_eq!(request.key, Key::from(MAX_AUTO_INCREMENT_KEY as i64));
    database.cancel_write(TX, request.id).unwrap();
    assert_eq!(
        put(&mut database, None, b"last", vec![], None).unwrap(),
        Key::from(MAX_AUTO_INCREMENT_KEY as i64)
    );
    assert!(matches!(
        database.prepare_write(TX, "a", None, false),
        Err(IndexedDbError::Constraint(_))
    ));
    assert!(database.is_transaction_active());
    database.abort(TX).unwrap();
    begin(&mut database, TransactionMode::ReadWrite);
    assert_eq!(
        put(&mut database, None, b"one again", vec![], None).unwrap(),
        Key::from(1)
    );
    database.commit(TX).unwrap();
}

#[test]
fn equal_zero_keys_preserve_record_representation_until_delete() {
    let mut database = database();
    begin(&mut database, TransactionMode::ReadWrite);
    let negative = Key::number(-0.0).unwrap();
    let positive = Key::number(0.0).unwrap();
    put(&mut database, Some(negative), b"first", vec![], None).unwrap();
    let returned = put(
        &mut database,
        Some(positive.clone()),
        b"second",
        vec![],
        None,
    )
    .unwrap();
    let Key::Number(returned) = returned else {
        panic!()
    };
    assert_eq!(returned.value().to_bits(), 0.0_f64.to_bits());
    let Key::Number(stored) = database.get_key(TX, "a", &positive).unwrap().unwrap() else {
        panic!()
    };
    assert_eq!(stored.value().to_bits(), (-0.0_f64).to_bits());
    database.delete(TX, "a", &positive).unwrap();
    put(
        &mut database,
        Some(positive.clone()),
        b"third",
        vec![],
        None,
    )
    .unwrap();
    let Key::Number(stored) = database.get_key(TX, "a", &positive).unwrap().unwrap() else {
        panic!()
    };
    assert_eq!(stored.value().to_bits(), 0.0_f64.to_bits());
    database.commit(TX).unwrap();
}

#[test]
fn upgrade_abort_restores_schema_data_generator_and_committed_usage() {
    let mut database = database();
    begin(&mut database, TransactionMode::ReadWrite);
    put(&mut database, None, b"original", vec![], None).unwrap();
    database.commit(TX).unwrap();
    let original = database.committed().clone();
    database
        .begin(TX, TransactionMode::VersionChange, BTreeSet::new(), Some(2))
        .unwrap();
    database.delete_object_store(TX, "a").unwrap();
    database
        .create_object_store(
            TX,
            "a",
            ObjectStoreOptions {
                auto_increment: true,
                ..Default::default()
            },
        )
        .unwrap();
    put(&mut database, None, b"replacement", vec![], None).unwrap();
    database.abort(TX).unwrap();
    assert_eq!(database.committed(), &original);
    begin(&mut database, TransactionMode::ReadWrite);
    assert_eq!(
        database
            .get(TX, "a", &1.into())
            .unwrap()
            .unwrap()
            .wire_bytes(),
        b"original"
    );
    assert_eq!(
        put(&mut database, None, b"next", vec![], None).unwrap(),
        Key::from(2)
    );
    database.commit(TX).unwrap();
}

#[test]
fn sqlite_automatic_rollback_is_fatal_instead_of_a_recoverable_request_error() {
    let mut database = database();
    // Install outside the tested transaction so the injected rollback does not
    // also roll back the trigger's creation.
    database.connection.execute_batch("CREATE TEMP TRIGGER fail_write BEFORE INSERT ON main.record_chunks BEGIN SELECT RAISE(ROLLBACK, 'injected transaction rollback'); END").unwrap();
    begin(&mut database, TransactionMode::ReadWrite);
    // A zero-length value has no record_chunks row and precedes the fault.
    put(&mut database, None, b"", vec![], None).unwrap();
    assert!(matches!(
        put(&mut database, None, b"failure", vec![], None),
        Err(IndexedDbError::TransactionInactive(_))
    ));
    assert!(!database.is_transaction_active());
    assert!(matches!(
        database.count(TX, "a"),
        Err(IndexedDbError::TransactionInactive(_))
    ));
    database
        .connection
        .execute_batch("DROP TRIGGER fail_write")
        .unwrap();
    begin(&mut database, TransactionMode::ReadWrite);
    assert_eq!(database.count(TX, "a").unwrap(), 0);
    assert_eq!(
        put(&mut database, None, b"fresh", vec![], None).unwrap(),
        Key::from(1)
    );
    database.commit(TX).unwrap();
}

#[test]
fn failed_single_command_restores_deleted_values_indexes_and_usage() {
    let mut database = database();
    add_unique_index(&mut database);
    begin(&mut database, TransactionMode::ReadWrite);
    let value = IndexedDbValue::new(
        b"wire".to_vec(),
        vec![External::Blob {
            bytes: b"blob".to_vec(),
            mime_type: "text/plain".into(),
        }],
    );
    let request = database.prepare_write(TX, "a", None, false).unwrap();
    database
        .finish_write(TX, request.id, value.clone(), index_keys("first"), None)
        .unwrap();
    database.commit(TX).unwrap();
    database.connection.execute_batch("CREATE TEMP TRIGGER fail_delete BEFORE DELETE ON main.records BEGIN SELECT RAISE(ABORT, 'injected request failure'); END").unwrap();

    begin(&mut database, TransactionMode::ReadWrite);
    put(&mut database, None, b"second", index_keys("second"), None).unwrap();
    let usage = database.candidate_metadata(TX).unwrap().usage;
    let range = crate::KeyRange {
        lower: Some(1.into()),
        upper: Some(1.into()),
        ..Default::default()
    };
    assert!(matches!(
        database.delete_range(TX, "a", &range),
        Err(IndexedDbError::Constraint(_))
    ));
    assert!(database.is_transaction_active());
    assert_eq!(database.candidate_metadata(TX).unwrap().usage, usage);
    assert_eq!(database.get(TX, "a", &1.into()).unwrap(), Some(value));
    assert_eq!(database.count(TX, "a").unwrap(), 2);
    assert_eq!(
        database
            .query_count(
                TX,
                "a",
                &QuerySource::Index("unique".into()),
                &Default::default()
            )
            .unwrap(),
        2
    );

    database
        .connection
        .execute_batch("DROP TRIGGER fail_delete")
        .unwrap();
    database.delete_range(TX, "a", &range).unwrap();
    database.commit(TX).unwrap();
    begin(&mut database, TransactionMode::ReadOnly);
    assert_eq!(database.count(TX, "a").unwrap(), 1);
    assert_eq!(
        database.get(TX, "a", &2.into()).unwrap(),
        Some(b"second".to_vec().into())
    );
    database.commit(TX).unwrap();
}

#[test]
fn single_command_automatic_rollback_aborts_prior_requests() {
    let mut database = database();
    database.connection.execute_batch("CREATE TEMP TRIGGER fail_delete BEFORE DELETE ON main.records BEGIN SELECT RAISE(ROLLBACK, 'injected transaction rollback'); END").unwrap();
    begin(&mut database, TransactionMode::ReadWrite);
    put(&mut database, None, b"first", vec![], None).unwrap();
    assert!(matches!(
        database.clear(TX, "a"),
        Err(IndexedDbError::TransactionInactive(_))
    ));
    assert!(!database.is_transaction_active());
    assert!(matches!(
        database.commit(TX),
        Err(IndexedDbError::TransactionInactive(_))
    ));
    begin(&mut database, TransactionMode::ReadWrite);
    assert_eq!(database.count(TX, "a").unwrap(), 0);
    assert_eq!(
        put(&mut database, None, b"fresh", vec![], None).unwrap(),
        1.into()
    );
    database.commit(TX).unwrap();
}

#[test]
fn lost_cross_command_savepoint_aborts_on_release_or_rollback_failure() {
    for cancel in [false, true] {
        let mut database = database();
        begin(&mut database, TransactionMode::ReadWrite);
        put(&mut database, None, b"first", vec![], None).unwrap();
        let request = database.prepare_write(TX, "a", None, false).unwrap();
        // Inject loss of the retained savepoint without ending the transaction.
        database
            .connection
            .execute_batch("RELEASE idb_request")
            .unwrap();
        let result = if cancel {
            database.cancel_write(TX, request.id)
        } else {
            database
                .finish_write(TX, request.id, b"second".to_vec().into(), vec![], None)
                .map(|_| ())
        };
        assert!(matches!(
            result,
            Err(IndexedDbError::TransactionInactive(_))
        ));
        assert!(!database.is_transaction_active());
        begin(&mut database, TransactionMode::ReadWrite);
        assert_eq!(database.count(TX, "a").unwrap(), 0);
        assert_eq!(
            put(&mut database, None, b"fresh", vec![], None).unwrap(),
            1.into()
        );
        database.commit(TX).unwrap();
    }
}

#[test]
fn chunked_values_roundtrip_and_delivered_bytes_survive_abort_and_close() {
    let mut database = database();
    begin(&mut database, TransactionMode::ReadWrite);
    let value = IndexedDbValue::new(
        vec![13; CHUNK_BYTES * 2 + 7],
        vec![
            External::Blob {
                bytes: vec![],
                mime_type: "empty/type".into(),
            },
            External::File {
                bytes: vec![17; CHUNK_BYTES + 19],
                mime_type: "file/type".into(),
                name: "file.name".into(),
                last_modified: f64::from_bits(0x7ff8_0000_0000_002a),
            },
            External::FileSystemHandle {
                kind: crate::IndexedDbFileSystemHandleKind::Directory,
                bucket: crate::IndexedDbFileSystemHandleBucket::Named { bucket_id: 17 },
                path: vec!["root".into(), "file".into()],
            },
        ],
    );
    let request = database.prepare_write(TX, "a", None, false).unwrap();
    database
        .finish_write(TX, request.id, value.clone(), vec![], None)
        .unwrap();
    let delivered = database.get(TX, "a", &1.into()).unwrap().unwrap();
    assert_eq!(delivered.wire_bytes(), value.wire_bytes());
    assert_eq!(
        &delivered.external_objects()[0],
        &value.external_objects()[0]
    );
    assert_eq!(
        &delivered.external_objects()[2],
        &value.external_objects()[2]
    );
    let External::File {
        bytes,
        name,
        last_modified,
        ..
    } = &delivered.external_objects()[1]
    else {
        panic!()
    };
    assert_eq!(bytes, &vec![17; CHUNK_BYTES + 19]);
    assert_eq!(name, "file.name");
    assert_eq!(last_modified.to_bits(), 0x7ff8_0000_0000_002a);
    database.abort(TX).unwrap();
    database.close().unwrap();
    assert_eq!(delivered.wire_bytes(), &vec![13; CHUNK_BYTES * 2 + 7]);
}

#[test]
fn key_only_queries_do_not_decode_corrupt_wire_chunks() {
    let mut database = database();
    begin(&mut database, TransactionMode::ReadWrite);
    put(&mut database, None, b"payload", vec![], None).unwrap();
    database
        .connection
        .execute("DELETE FROM record_chunks", [])
        .unwrap();
    assert_eq!(
        database.get_key(TX, "a", &1.into()).unwrap(),
        Some(1.into())
    );
    assert_eq!(database.count(TX, "a").unwrap(), 1);
    assert!(matches!(
        database.get(TX, "a", &1.into()),
        Err(IndexedDbError::Corruption(_))
    ));
    database.abort(TX).unwrap();
}

#[test]
fn clearing_records_reclaims_chunks_slots_and_indexes_without_resetting_generator() {
    let mut database = database();
    add_unique_index(&mut database);
    let base_usage = database.committed().usage;
    begin(&mut database, TransactionMode::ReadWrite);
    let request = database.prepare_write(TX, "a", None, false).unwrap();
    database
        .finish_write(
            TX,
            request.id,
            IndexedDbValue::new(
                vec![1],
                vec![External::Blob {
                    bytes: vec![2; CHUNK_BYTES + 1],
                    mime_type: "x".into(),
                }],
            ),
            index_keys("saved"),
            None,
        )
        .unwrap();
    database.clear(TX, "a").unwrap();
    for table in [
        "records",
        "record_chunks",
        "index_entries",
        "record_objects",
        "external_objects",
        "blob_chunks",
        "handle_components",
    ] {
        assert_eq!(
            database
                .connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0,
            "{table}"
        );
    }
    assert_eq!(database.candidate_metadata(TX).unwrap().usage, base_usage);
    assert_eq!(
        put(&mut database, None, b"two", index_keys("new"), None).unwrap(),
        Key::from(2)
    );
    database.commit(TX).unwrap();
}

#[test]
fn index_backfill_checks_uniqueness_and_upgrade_abort_removes_partial_index() {
    let mut database = database();
    begin(&mut database, TransactionMode::ReadWrite);
    put(&mut database, None, b"one", vec![], None).unwrap();
    put(&mut database, None, b"two", vec![], None).unwrap();
    database.commit(TX).unwrap();
    let original = database.committed().clone();
    database
        .begin(TX, TransactionMode::VersionChange, BTreeSet::new(), Some(2))
        .unwrap();
    database
        .create_index(
            TX,
            "a",
            "unique",
            IndexOptions {
                key_path: "value".into(),
                unique: true,
                multi_entry: false,
            },
        )
        .unwrap();
    database
        .backfill_index_record(TX, "a", "unique", &1.into(), vec!["same".into()])
        .unwrap();
    assert!(matches!(
        database.backfill_index_record(TX, "a", "unique", &2.into(), vec!["same".into()]),
        Err(IndexedDbError::Constraint(_))
    ));
    database.abort(TX).unwrap();
    assert_eq!(database.committed(), &original);
    assert_eq!(
        database
            .connection
            .query_row("SELECT count(*) FROM indexes", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        database
            .connection
            .query_row("SELECT count(*) FROM index_entries", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn integer_overflow_and_stale_generation_are_rejected_without_publishing_data() {
    let mut database = empty();
    assert!(matches!(
        database.begin(
            TX,
            TransactionMode::VersionChange,
            BTreeSet::new(),
            Some(u64::MAX)
        ),
        Err(IndexedDbError::Data(_))
    ));
    assert_eq!(database.committed().version, 0);
    let connection = database.connection;
    let mut wrong = identity();
    wrong.generation[0] ^= 1;
    assert!(matches!(
        Database::open(connection, wrong),
        Err(IndexedDbError::Corruption(_))
    ));
}

#[test]
fn multi_entry_deduplication_uses_key_equality_and_caller_cannot_disable_uniqueness() {
    let mut database = database();
    database
        .begin(TX, TransactionMode::VersionChange, BTreeSet::new(), Some(2))
        .unwrap();
    database
        .create_index(
            TX,
            "a",
            "multi",
            IndexOptions {
                key_path: "values".into(),
                unique: true,
                multi_entry: true,
            },
        )
        .unwrap();
    database.commit(TX).unwrap();
    begin(&mut database, TransactionMode::ReadWrite);
    let keys = || {
        vec![IndexKeys {
            name: "multi".into(),
            keys: vec![
                Key::number(-0.0).unwrap(),
                Key::number(0.0).unwrap(),
                "another".into(),
                "another".into(),
            ],
        }]
    };
    put(&mut database, None, b"one", keys(), None).unwrap();
    assert_eq!(
        database
            .connection
            .query_row("SELECT count(*) FROM index_entries", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert!(matches!(
        put(&mut database, None, b"two", keys(), None),
        Err(IndexedDbError::Constraint(_))
    ));
    assert_eq!(database.count(TX, "a").unwrap(), 1);
    database.commit(TX).unwrap();
}

#[test]
fn missing_external_slots_and_noncontiguous_chunks_are_reported_as_corruption() {
    let mut database = database();
    begin(&mut database, TransactionMode::ReadWrite);
    let request = database.prepare_write(TX, "a", None, false).unwrap();
    database
        .finish_write(
            TX,
            request.id,
            IndexedDbValue::new(
                vec![1; CHUNK_BYTES + 1],
                vec![External::Blob {
                    bytes: vec![2],
                    mime_type: "x".into(),
                }],
            ),
            vec![],
            None,
        )
        .unwrap();
    database.commit(TX).unwrap();
    for damage in [
        "DELETE FROM external_objects",
        "UPDATE record_objects SET ordinal=1",
        "DELETE FROM record_chunks WHERE chunk_no=0",
        "UPDATE record_chunks SET bytes=x'00' WHERE chunk_no=0",
    ] {
        begin(&mut database, TransactionMode::ReadWrite);
        database.connection.execute_batch(damage).unwrap();
        assert!(
            matches!(
                database.get(TX, "a", &1.into()),
                Err(IndexedDbError::Corruption(_))
            ),
            "{damage}"
        );
        database.abort(TX).unwrap();
    }
}

#[test]
fn disk_owner_recovers_committed_schema_values_and_high_water_mark() {
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let mut token = [0; 16];
    moli_crypto::fill_secure_random(&mut token).unwrap();
    let directory = Directory(
        std::env::temp_dir().join(format!("moli-idb-sql-{}", moli_crypto::sha256_hex(token))),
    );
    std::fs::create_dir(&directory.0).unwrap();
    let path = directory.0.join("db.sqlite");
    let connection =
        moli_sqlite::open_disk(&path, moli_sqlite::OpenMode::Create, Default::default()).unwrap();
    let mut database = Database::create(connection, identity()).unwrap();
    database
        .begin(TX, TransactionMode::VersionChange, BTreeSet::new(), Some(1))
        .unwrap();
    database
        .create_object_store(
            TX,
            "a",
            ObjectStoreOptions {
                auto_increment: true,
                ..Default::default()
            },
        )
        .unwrap();
    put(&mut database, None, b"persisted", vec![], None).unwrap();
    database.commit(TX).unwrap();
    let original = database.committed().clone();
    begin(&mut database, TransactionMode::ReadWrite);
    put(&mut database, None, b"abandoned", vec![], None).unwrap();
    database.close().unwrap();
    let connection =
        moli_sqlite::open_disk(&path, moli_sqlite::OpenMode::Existing, Default::default()).unwrap();
    let mut database = Database::open(connection, identity()).unwrap();
    assert_eq!(database.committed(), &original);
    begin(&mut database, TransactionMode::ReadWrite);
    assert_eq!(database.count(TX, "a").unwrap(), 1);
    assert_eq!(
        database
            .get(TX, "a", &1.into())
            .unwrap()
            .unwrap()
            .wire_bytes(),
        b"persisted"
    );
    assert_eq!(
        put(&mut database, None, b"next", vec![], None).unwrap(),
        Key::from(2)
    );
    database.commit(TX).unwrap();
    database.close().unwrap();
    let connection =
        moli_sqlite::open_disk(&path, moli_sqlite::OpenMode::Existing, Default::default()).unwrap();
    let mut wrong = identity();
    wrong.owner_key.push(0);
    assert!(matches!(
        Database::open(connection, wrong),
        Err(IndexedDbError::Corruption(_))
    ));
    let connection =
        moli_sqlite::open_disk(&path, moli_sqlite::OpenMode::Existing, Default::default()).unwrap();
    let database = Database::open(connection, identity()).unwrap();
    assert_eq!(database.committed().usage, original.usage + 8 + 4);
    database.close().unwrap();
}
