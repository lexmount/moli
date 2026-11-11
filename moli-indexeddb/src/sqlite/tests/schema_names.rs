use super::*;
use crate::IndexedDbName;

struct Directory(std::path::PathBuf);
impl Directory {
    fn new() -> Self {
        let mut token = [0; 16];
        moli_crypto::fill_secure_random(&mut token).unwrap();
        let directory = Self(std::env::temp_dir().join(format!(
            "moli-idb-schema-names-{}",
            moli_crypto::sha256_hex(token)
        )));
        std::fs::create_dir(&directory.0).unwrap();
        directory
    }
    fn path(&self) -> std::path::PathBuf {
        self.0.join("database.sqlite")
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn lossless_store_and_index_names_preserve_separate_records_across_disk_restart() {
    let directory = Directory::new();
    let names = [
        vec![],
        vec![0],
        vec![0xd800],
        vec![0xd801],
        vec![0xdc00],
        vec![0xfffd],
        vec![0xdc00, 0xd800],
        vec![0xd83d, 0xde00],
        vec![0xe000],
    ]
    .into_iter()
    .map(IndexedDbName::from_utf16)
    .collect::<Vec<_>>();
    let connection = moli_sqlite::open_disk(
        &directory.path(),
        moli_sqlite::OpenMode::Create,
        Default::default(),
    )
    .unwrap();
    let mut database = Database::create(connection, identity()).unwrap();
    database
        .begin(TX, TransactionMode::VersionChange, BTreeSet::new(), Some(1))
        .unwrap();
    for (ordinal, name) in names.iter().enumerate() {
        database
            .create_object_store(
                TX,
                database.allocate_store_id().unwrap(),
                name,
                ObjectStoreOptions {
                    auto_increment: true,
                    ..Default::default()
                },
            )
            .unwrap();
        for index in &names {
            database
                .create_index(
                    TX,
                    database.lookup_store_id(name).unwrap(),
                    database.allocate_index_id().unwrap(),
                    index,
                    IndexOptions {
                        key_path: KeyPath::String("label".into()),
                        unique: true,
                        multi_entry: false,
                    },
                )
                .unwrap();
        }
        database
            .write(
                TX,
                database.lookup_store_id(name).unwrap(),
                RecordWrite {
                    key: None,
                    value: vec![ordinal as u8].into(),
                    indexes: names
                        .iter()
                        .map(|index_name| IndexKeys {
                            index: database
                                .lookup_index_id(
                                    database.lookup_store_id(name).unwrap(),
                                    index_name,
                                )
                                .unwrap(),
                            keys: vec![(ordinal as i64).into()],
                        })
                        .collect(),
                    add_only: true,
                    quota: None,
                },
            )
            .unwrap();
    }
    let original = database.commit(TX).unwrap();
    database.close().unwrap();
    let connection = moli_sqlite::open_disk(
        &directory.path(),
        moli_sqlite::OpenMode::Existing,
        Default::default(),
    )
    .unwrap();
    let mut database = Database::open(connection, identity()).unwrap();
    assert_eq!(database.committed(), &original);
    database
        .begin(
            TX,
            TransactionMode::ReadOnly,
            names
                .iter()
                .cloned()
                .map(|name: crate::IndexedDbName| database.lookup_store_id(name).unwrap())
                .collect(),
            None,
        )
        .unwrap();
    for (ordinal, name) in names.iter().enumerate() {
        for index in &names {
            let entries = database
                .query(
                    TX,
                    database.lookup_store_id(name).unwrap(),
                    &Query {
                        source: QuerySource::Index(
                            database
                                .lookup_index_id(
                                    database.lookup_store_id(name).unwrap(),
                                    index.clone(),
                                )
                                .unwrap(),
                        ),
                        ..Default::default()
                    },
                )
                .unwrap();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].key, (ordinal as i64).into());
            assert_eq!(
                entries[0].value.as_ref().unwrap().wire_bytes(),
                &[ordinal as u8]
            );
        }
    }
    database.commit(TX).unwrap();
    database
        .begin(TX, TransactionMode::VersionChange, BTreeSet::new(), Some(2))
        .unwrap();
    for (ordinal, name) in names.iter().enumerate() {
        let temporary = format!("temporary-{ordinal}");
        database
            .rename_object_store(
                TX,
                database.lookup_store_id(name).unwrap(),
                temporary.as_str(),
            )
            .unwrap();
        database
            .rename_index(
                TX,
                database.lookup_store_id(temporary.as_str()).unwrap(),
                database
                    .lookup_index_id(database.lookup_store_id(temporary.as_str()).unwrap(), name)
                    .unwrap(),
                "temporary-index",
            )
            .unwrap();
    }
    database.abort(TX).unwrap();
    assert_eq!(database.committed(), &original);
    database.close().unwrap();
}

#[test]
fn v2_name_migration_rolls_back_all_ddl_on_a_malformed_label_and_can_retry() {
    let directory = Directory::new();
    let connection = moli_sqlite::open_disk(
        &directory.path(),
        moli_sqlite::OpenMode::Create,
        Default::default(),
    )
    .unwrap();
    connection
        .execute_batch(include_str!("fixtures/schema-v2.sql"))
        .unwrap();
    let identity = identity();
    connection
        .execute(
            "INSERT INTO database_info VALUES (1,'indexeddb',?1,1)",
            [&identity.owner_key],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO database_metadata VALUES (1,?1,?2,1,'live',1,1,1,128,4,10,1,1)",
            rusqlite::params![
                names::encode(&identity.name).unwrap(),
                identity.generation.as_slice()
            ],
        )
        .unwrap();
    connection
        .execute("INSERT INTO object_stores VALUES (3,'legacy',NULL,1,7)", [])
        .unwrap();
    connection
        .execute(
            "INSERT INTO indexes VALUES (9,3,CAST(X'FF' AS TEXT),?1,0,0)",
            [KeyPath::String("label".into()).encode_value().unwrap()],
        )
        .unwrap();
    connection.close().unwrap();
    let connection = moli_sqlite::open_disk(
        &directory.path(),
        moli_sqlite::OpenMode::Existing,
        Default::default(),
    )
    .unwrap();
    assert!(matches!(
        Database::open(connection, identity.clone()),
        Err(IndexedDbError::Corruption(_))
    ));
    let connection = moli_sqlite::open_disk(
        &directory.path(),
        moli_sqlite::OpenMode::Existing,
        Default::default(),
    )
    .unwrap();
    assert_eq!(names::schema_version(&connection).unwrap(), 2);
    assert_eq!(
        connection
            .query_row("SELECT typeof(name) FROM object_stores", [], |row| row
                .get::<_, String>(
                0
            ))
            .unwrap(),
        "text"
    );
    assert_eq!(connection.query_row("SELECT count(*) FROM sqlite_schema WHERE name IN ('object_stores_v2','indexes_v2')", [], |row| row.get::<_,i64>(0)).unwrap(), 0);
    connection
        .execute("UPDATE indexes SET name='valid'", [])
        .unwrap();
    connection.close().unwrap();
    let connection = moli_sqlite::open_disk(
        &directory.path(),
        moli_sqlite::OpenMode::Existing,
        Default::default(),
    )
    .unwrap();
    let mut database = Database::open(connection, identity).unwrap();
    assert_eq!(
        names::schema_version(&database.connection).unwrap(),
        SCHEMA_VERSION
    );
    assert_eq!(
        database.committed().stores[&IndexedDbName::from("legacy")].id,
        StoreId::from_raw(3)
    );
    assert_eq!(
        database.committed().stores[&IndexedDbName::from("legacy")].indexes
            [&IndexedDbName::from("valid")]
            .id,
        IndexId::from_raw(9)
    );
    database
        .begin(
            TX,
            TransactionMode::ReadWrite,
            (BTreeSet::from(["legacy".into()]))
                .into_iter()
                .map(|name: crate::IndexedDbName| database.lookup_store_id(name).unwrap())
                .collect(),
            None,
        )
        .unwrap();
    assert_eq!(
        database
            .write(
                TX,
                database.lookup_store_id("legacy").unwrap(),
                RecordWrite {
                    key: None,
                    value: b"kept".to_vec().into(),
                    indexes: vec![IndexKeys {
                        index: database
                            .lookup_index_id(database.lookup_store_id("legacy").unwrap(), "valid")
                            .unwrap(),
                        keys: vec![1.into()]
                    }],
                    add_only: true,
                    quota: None,
                }
            )
            .unwrap(),
        8.into()
    );
    database.commit(TX).unwrap();
    database.close().unwrap();
}
