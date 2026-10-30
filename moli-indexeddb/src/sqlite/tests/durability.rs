use super::*;

fn synchronous(database: &Database) -> i64 {
    database
        .connection
        .pragma_query_value(None, "synchronous", |row| row.get(0))
        .unwrap()
}

#[test]
fn writer_durability_switches_before_begin_and_readers_keep_the_active_policy() {
    let mut database = database();
    for (policy, expected) in [
        (TransactionDurability::Strict, 2),
        (TransactionDurability::Relaxed, 1),
    ] {
        database
            .begin_with_durability(
                TX,
                TransactionMode::ReadWrite,
                BTreeSet::from(["a".into()]),
                None,
                policy,
            )
            .unwrap();
        assert_eq!(synchronous(&database), expected);
        let reader = TransactionHandle::from_raw(2);
        database
            .begin_with_durability(
                reader,
                TransactionMode::ReadOnly,
                BTreeSet::from(["b".into()]),
                None,
                TransactionDurability::Relaxed,
            )
            .unwrap();
        assert_eq!(synchronous(&database), expected);
        database.commit(reader).unwrap();
        // A rejected second writer must neither change PRAGMA nor disturb the
        // active transaction, even when its requested hint differs.
        let other = if expected == 2 {
            TransactionDurability::Relaxed
        } else {
            TransactionDurability::Strict
        };
        assert!(matches!(
            database.begin_with_durability(
                reader,
                TransactionMode::ReadWrite,
                BTreeSet::from(["b".into()]),
                None,
                other
            ),
            Err(IndexedDbError::InvalidState(_))
        ));
        assert_eq!(synchronous(&database), expected);
        put(&mut database, Some(expected.into()), b"kept", vec![], None).unwrap();
        database.commit(TX).unwrap();
    }
    // Even without a writer, a readonly hint does not reconfigure the owner.
    database
        .begin_with_durability(
            TX,
            TransactionMode::ReadOnly,
            BTreeSet::from(["a".into()]),
            None,
            TransactionDurability::Strict,
        )
        .unwrap();
    assert_eq!(synchronous(&database), 1);
    database.commit(TX).unwrap();
}

#[test]
fn invalid_begin_does_not_change_the_last_successful_writer_policy() {
    let mut database = database();
    database
        .begin_with_durability(
            TX,
            TransactionMode::ReadWrite,
            BTreeSet::from(["a".into()]),
            None,
            TransactionDurability::Strict,
        )
        .unwrap();
    database.abort(TX).unwrap();
    assert!(matches!(
        database.begin_with_durability(
            TX,
            TransactionMode::ReadWrite,
            BTreeSet::from(["missing".into()]),
            None,
            TransactionDurability::Relaxed
        ),
        Err(IndexedDbError::NotFound(_))
    ));
    assert_eq!(synchronous(&database), 2);
    assert!(matches!(
        database.begin_with_durability(
            TX,
            TransactionMode::VersionChange,
            BTreeSet::new(),
            Some(1),
            TransactionDurability::Relaxed
        ),
        Err(IndexedDbError::Version(_))
    ));
    assert_eq!(synchronous(&database), 2);
    assert!(database.connection.is_autocommit());
}

#[test]
fn strict_and_relaxed_commits_survive_disk_reopen_and_aborts_stay_rolled_back() {
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let mut token = [0; 16];
    moli_crypto::fill_secure_random(&mut token).unwrap();
    let directory = Directory(std::env::temp_dir().join(format!(
        "moli-idb-durability-{}",
        moli_crypto::sha256_hex(token)
    )));
    std::fs::create_dir(&directory.0).unwrap();
    let path = directory.0.join("database.sqlite");
    {
        let connection =
            moli_sqlite::open_disk(&path, moli_sqlite::OpenMode::Create, Default::default())
                .unwrap();
        let mut database = Database::create(connection, identity()).unwrap();
        database
            .begin_with_durability(
                TX,
                TransactionMode::VersionChange,
                BTreeSet::new(),
                Some(1),
                TransactionDurability::Strict,
            )
            .unwrap();
        assert_eq!(synchronous(&database), 2);
        database
            .create_object_store(TX, "a", ObjectStoreOptions::default())
            .unwrap();
        database.commit(TX).unwrap();
        for (key, policy) in [
            (1, TransactionDurability::Strict),
            (2, TransactionDurability::Relaxed),
        ] {
            database
                .begin_with_durability(
                    TX,
                    TransactionMode::ReadWrite,
                    BTreeSet::from(["a".into()]),
                    None,
                    policy,
                )
                .unwrap();
            put(&mut database, Some(key.into()), b"committed", vec![], None).unwrap();
            database.commit(TX).unwrap();
            database
                .begin_with_durability(
                    TX,
                    TransactionMode::ReadWrite,
                    BTreeSet::from(["a".into()]),
                    None,
                    policy,
                )
                .unwrap();
            put(
                &mut database,
                Some((key + 10).into()),
                b"aborted",
                vec![],
                None,
            )
            .unwrap();
            database.abort(TX).unwrap();
        }
        database.close().unwrap();
    }
    let connection =
        moli_sqlite::open_disk(&path, moli_sqlite::OpenMode::Existing, Default::default()).unwrap();
    let mut database = Database::open(connection, identity()).unwrap();
    begin(&mut database, TransactionMode::ReadOnly);
    for key in [1, 2] {
        assert_eq!(
            database
                .get(TX, "a", &key.into())
                .unwrap()
                .unwrap()
                .wire_bytes(),
            b"committed"
        );
        assert!(database.get(TX, "a", &(key + 10).into()).unwrap().is_none());
    }
    database.commit(TX).unwrap();
    database.close().unwrap();
}
