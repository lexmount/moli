use rusqlite::{Connection, params};

const SCHEMA: &str = include_str!("sqlite/schema.sql");

fn database() -> Connection {
    let mut connection = Connection::open_in_memory().unwrap();
    let transaction = connection.transaction().unwrap();
    transaction.execute_batch(SCHEMA).unwrap();
    transaction
        .execute(
            "INSERT INTO database_info VALUES (1, 'indexeddb', X'01', 1)",
            [],
        )
        .unwrap();
    transaction
        .execute(
            "INSERT INTO database_metadata
             (singleton, name, generation, version, state,
              key_format, key_path_format, value_format, usage)
             VALUES (1, ?1, zeroblob(16), 0, 'creating', 1, 1, 1, 0)",
            ["database"
                .encode_utf16()
                .flat_map(u16::to_be_bytes)
                .collect::<Vec<_>>()],
        )
        .unwrap();
    transaction.commit().unwrap();
    connection
}

#[test]
fn schema_uses_eleven_strict_without_rowid_tables_and_explicit_ids() {
    let connection = database();
    let tables = connection
        .prepare(
            "SELECT name, wr, strict FROM pragma_table_list
             WHERE schema = 'main' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(tables.len(), 11);
    assert!(
        tables
            .iter()
            .all(|(_, wr, strict)| *wr == 1 && *strict == 1)
    );
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        i64::from(crate::sqlite::SCHEMA_VERSION)
    );
    for statement in [
        "INSERT INTO object_stores(name, auto_increment, generator) VALUES (X'00730074006f00720065', 0, 0)",
        "INSERT INTO indexes(store_id, name, key_path, is_unique, multi_entry)
         VALUES (1, X'0069006e006400650078', X'00', 0, 0)",
        "INSERT INTO records(store_id, key_sort, key_value, wire_size, usage)
         VALUES (1, X'10', X'10', 0, 0)",
        "INSERT INTO external_objects(kind, size, mime_type) VALUES ('blob', 0, '')",
    ] {
        assert!(connection.execute(statement, []).is_err(), "{statement}");
    }
    let forbidden = connection
        .query_row(
            "SELECT count(*) FROM pragma_table_info('database_info')
             WHERE name IN ('import_state', 'source_format', 'source_digest')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    assert_eq!(forbidden, 0);
    assert_eq!(rusqlite::version(), "3.53.2");
}

#[test]
fn schema_preserves_inclusive_generator_boundary_and_metadata_state() {
    let connection = database();
    for generator in [0_i64, 9_007_199_254_740_991, 9_007_199_254_740_992] {
        connection
            .execute(
                "INSERT INTO object_stores VALUES (1, X'00730074006f00720065', NULL, 1, ?1)
                 ON CONFLICT(store_id) DO UPDATE SET generator = excluded.generator",
                [generator],
            )
            .unwrap();
    }
    for generator in [-1_i64, 9_007_199_254_740_993] {
        assert!(
            connection
                .execute("UPDATE object_stores SET generator = ?1", [generator])
                .is_err()
        );
    }
    assert!(
        connection
            .execute("UPDATE object_stores SET generator = 1.5", [])
            .is_err()
    );
    assert!(
        connection
            .execute("UPDATE database_metadata SET state = 'live'", [])
            .is_err()
    );
    connection
        .execute(
            "UPDATE database_metadata SET state = 'live', version = 1",
            [],
        )
        .unwrap();
    assert!(
        connection
            .execute("UPDATE database_metadata SET version = -1", [])
            .is_err()
    );
    assert!(
        connection
            .execute("UPDATE database_metadata SET generation = zeroblob(15)", [])
            .is_err()
    );
    assert!(
        connection
            .execute("UPDATE database_metadata SET next_store_id = 0", [])
            .is_err()
    );
}

fn index_entry(
    connection: &Connection,
    index: i64,
    primary_key: u8,
    unique: bool,
) -> rusqlite::Result<usize> {
    connection.execute(
        "INSERT INTO index_entries
         (index_id, index_key_sort, index_key_value, primary_key_sort, record_id, is_unique)
         VALUES (?1, X'10', X'10', ?2, ?3, ?4)",
        params![
            index,
            [primary_key].as_slice(),
            i64::from(primary_key),
            unique
        ],
    )
}

#[test]
fn schema_enforces_unique_indexes_and_keeps_nonunique_primary_order() {
    let connection = database();
    index_entry(&connection, 1, 1, true).unwrap();
    assert!(index_entry(&connection, 1, 2, true).is_err());
    index_entry(&connection, 2, 2, false).unwrap();
    index_entry(&connection, 2, 1, false).unwrap();
    assert!(index_entry(&connection, 2, 1, false).is_err());
    let primary = connection
        .query_row(
            "SELECT primary_key_sort FROM index_entries
             WHERE index_id = 2 AND index_key_sort = X'10'
             ORDER BY primary_key_sort LIMIT 1",
            [],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .unwrap();
    assert_eq!(primary, [1]);
    let query_plan = connection
        .prepare(
            "EXPLAIN QUERY PLAN SELECT primary_key_sort FROM index_entries
             WHERE index_id = 2 AND index_key_sort < X'20'
             ORDER BY index_key_sort DESC, primary_key_sort DESC LIMIT 1",
        )
        .unwrap()
        .query_map([], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        query_plan
            .iter()
            .any(|line| line.contains("SEARCH") && line.contains("PRIMARY KEY"))
    );
    assert!(
        query_plan
            .iter()
            .all(|line| !line.contains("TEMP B-TREE") && !line.contains("SCAN"))
    );
}

#[test]
fn schema_rejects_invalid_external_objects_and_unbounded_chunks() {
    let connection = database();
    for statement in [
        "INSERT INTO external_objects(object_id, kind, size) VALUES (1, 'blob', 0)",
        "INSERT INTO external_objects(object_id, kind, size, mime_type)
         VALUES (1, 'file', 0, '')",
        "INSERT INTO external_objects(object_id, kind, size, handle_kind, handle_bucket_kind)
         VALUES (1, 'file-system-handle', 0, 'file', 'named')",
        "INSERT INTO external_objects(object_id, kind, size, handle_kind, handle_bucket_kind, handle_bucket_id)
         VALUES (1, 'file-system-handle', 0, 'file', 'default', 1)",
        "INSERT INTO blob_chunks VALUES (1, 0, zeroblob(0))",
        "INSERT INTO record_chunks VALUES (1, 0, zeroblob(262145))",
        "INSERT INTO record_objects VALUES (1, -1, 1)",
    ] {
        assert!(connection.execute(statement, []).is_err(), "{statement}");
    }
    let bits = f64::NAN.to_bits().to_le_bytes();
    connection
        .execute(
            "INSERT INTO external_objects
             (object_id, kind, size, mime_type, file_name, last_modified_f64)
             VALUES (1, 'file', 0, '', 'file', ?1)",
            [bits.as_slice()],
        )
        .unwrap();
    let stored = connection
        .query_row(
            "SELECT last_modified_f64 FROM external_objects",
            [],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .unwrap();
    assert_eq!(stored, bits);
    connection
        .execute(
            "INSERT INTO record_chunks VALUES (1, 0, zeroblob(262144))",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO external_objects
             (object_id, kind, size, handle_kind, handle_bucket_kind)
             VALUES (2, 'file-system-handle', 0, 'directory', 'default')",
            [],
        )
        .unwrap();
}

#[test]
fn schema_database_names_are_even_utf16_blobs_and_preserve_lone_surrogates() {
    let connection = database();
    assert!(
        connection
            .execute("UPDATE database_metadata SET name='text'", [])
            .is_err()
    );
    assert!(
        connection
            .execute("UPDATE database_metadata SET name=X'D8'", [])
            .is_err()
    );
    connection
        .execute("UPDATE database_metadata SET name=X'D800'", [])
        .unwrap();
    assert_eq!(
        connection
            .query_row("SELECT name FROM database_metadata", [], |row| row
                .get::<_, Vec<u8>>(0))
            .unwrap(),
        vec![0xd8, 0]
    );
}
