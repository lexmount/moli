use super::*;
use crate::IndexedDbName;

struct Directory(std::path::PathBuf);
impl Directory {
    fn new() -> Self {
        let mut token = [0; 16];
        moli_crypto::fill_secure_random(&mut token).unwrap();
        let path = std::env::temp_dir().join(format!(
            "moli-idb-name-migration-{}",
            moli_crypto::sha256_hex(token)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
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

fn legacy_database(directory: &Directory) -> DatabaseIdentity {
    let mut identity = identity();
    identity.name = "legacy-\0-数据库-\u{1f4be}".into();
    let mut connection = moli_sqlite::open_disk(
        &directory.path(),
        moli_sqlite::OpenMode::Create,
        Default::default(),
    )
    .unwrap();
    let transaction = connection.transaction().unwrap();
    transaction
        .execute_batch(include_str!("fixtures/schema-v1.sql"))
        .unwrap();
    transaction
        .execute(
            "INSERT INTO database_info VALUES(1, 'indexeddb', ?1, 1)",
            [&identity.owner_key],
        )
        .unwrap();
    let options = ObjectStoreOptions {
        auto_increment: true,
        ..Default::default()
    };
    let usage = identity.name.usage_bytes() + 8 + schema::store_usage(&"a".into(), &options) + 12;
    transaction.execute(
        "INSERT INTO database_metadata(singleton,name,generation,version,state,key_format,key_path_format,value_format,usage,next_store_id,next_record_id) VALUES(1,?1,?2,1,'live',1,1,1,?3,4,8)",
        rusqlite::params![String::from_utf16(identity.name.as_utf16()).unwrap(), identity.generation.as_slice(), usage as i64],
    ).unwrap();
    transaction
        .execute("INSERT INTO object_stores VALUES(3,'a',NULL,1,7)", [])
        .unwrap();
    let key = Key::from(5);
    transaction
        .execute(
            "INSERT INTO records VALUES(7,3,?1,?2,4,12)",
            rusqlite::params![key.encode_ordered().unwrap(), key.encode_value().unwrap()],
        )
        .unwrap();
    transaction
        .execute(
            "INSERT INTO record_chunks VALUES(7,0,?1)",
            [b"keep".as_slice()],
        )
        .unwrap();
    transaction.commit().unwrap();
    identity
}

#[test]
fn database_name_codec_preserves_code_units_and_rejects_malformed_payloads() {
    for units in [
        vec![],
        vec![0],
        vec![0xd800],
        vec![0xd801],
        vec![0xfffd],
        vec![0xd800, 0xdc00],
        vec![0xdc00, 0xd800],
        vec![0xd800, 0],
    ] {
        let name = IndexedDbName::from_utf16(units.clone());
        let bytes = names::encode(&name).unwrap();
        assert_eq!(names::decode(&bytes).unwrap().as_utf16(), units);
        let mut database = Database::create(
            moli_sqlite::open_private_memory(Default::default()).unwrap(),
            DatabaseIdentity {
                name: name.clone(),
                ..identity()
            },
        )
        .unwrap();
        database
            .begin(TX, TransactionMode::VersionChange, BTreeSet::new(), Some(1))
            .unwrap();
        database.commit(TX).unwrap();
        assert_eq!(database.committed().name, name);
        let stored: Vec<u8> = database
            .connection
            .query_row("SELECT name FROM database_metadata", [], |row| row.get(0))
            .unwrap();
        assert_eq!(stored, bytes);
    }
    assert!(names::decode(&[0xd8]).is_err());
}

#[test]
fn legacy_name_migration_preserves_records_ids_generator_and_following_commits() {
    let directory = Directory::new();
    let identity = legacy_database(&directory);
    let connection = moli_sqlite::open_disk(
        &directory.path(),
        moli_sqlite::OpenMode::Existing,
        Default::default(),
    )
    .unwrap();
    let mut database = Database::open(connection, identity.clone()).unwrap();
    assert_eq!(database.committed().name, identity.name);
    assert_eq!(
        names::schema_version(&database.connection).unwrap(),
        SCHEMA_VERSION
    );
    assert_eq!(
        database.committed().stores[&crate::IndexedDbName::from("a")].id,
        3
    );
    begin(&mut database, TransactionMode::ReadWrite);
    assert_eq!(
        database
            .get(TX, "a", &5.into())
            .unwrap()
            .unwrap()
            .wire_bytes(),
        b"keep"
    );
    assert_eq!(
        put(&mut database, None, b"new", vec![], None).unwrap(),
        8.into()
    );
    database.commit(TX).unwrap();
    database.close().unwrap();
    let connection = moli_sqlite::open_disk(
        &directory.path(),
        moli_sqlite::OpenMode::Existing,
        Default::default(),
    )
    .unwrap();
    let mut database = Database::open(connection, identity).unwrap();
    begin(&mut database, TransactionMode::ReadOnly);
    assert_eq!(database.count(TX, "a").unwrap(), 2);
    assert_eq!(
        database
            .get(TX, "a", &8.into())
            .unwrap()
            .unwrap()
            .wire_bytes(),
        b"new"
    );
    database.commit(TX).unwrap();
}

#[test]
fn failed_name_migration_keeps_legacy_format_and_data_available_for_reopen() {
    let directory = Directory::new();
    let identity = legacy_database(&directory);
    let connection =
        Connection::open_with_flags(directory.path(), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    assert!(Database::open(connection, identity.clone()).is_err());
    let connection = Connection::open(directory.path()).unwrap();
    assert_eq!(names::schema_version(&connection).unwrap(), 1);
    assert_eq!(
        connection
            .query_row("SELECT typeof(name) FROM database_metadata", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap(),
        "text"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name='database_metadata_v1'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT bytes FROM record_chunks WHERE record_id=7",
                [],
                |row| row.get::<_, Vec<u8>>(0)
            )
            .unwrap(),
        b"keep"
    );
    drop(connection);
    let connection = moli_sqlite::open_disk(
        &directory.path(),
        moli_sqlite::OpenMode::Existing,
        Default::default(),
    )
    .unwrap();
    assert_eq!(
        Database::open(connection, identity)
            .unwrap()
            .committed()
            .version,
        1
    );
}

#[test]
fn foreign_legacy_identity_is_rejected_before_migration() {
    let directory = Directory::new();
    let mut identity = legacy_database(&directory);
    identity.owner_key.push(0);
    let connection = Connection::open(directory.path()).unwrap();
    assert!(Database::open(connection, identity).is_err());
    let connection = Connection::open(directory.path()).unwrap();
    assert_eq!(names::schema_version(&connection).unwrap(), 1);
    assert_eq!(
        connection
            .query_row("SELECT typeof(name) FROM database_metadata", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap(),
        "text"
    );
}
