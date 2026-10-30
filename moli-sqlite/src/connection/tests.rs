use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use super::*;

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "moli-sqlite-{}-{}",
            std::process::id(),
            NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn database(&self) -> PathBuf {
        self.0.join("db.sqlite")
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn pragma_string(connection: &Connection, pragma: &str) -> String {
    connection
        .pragma_query_value(None, pragma, |row| row.get(0))
        .unwrap()
}

#[test]
fn disk_policy_is_effective_and_committed_data_survives_reopen() {
    let directory = TestDirectory::new();
    let path = directory.database();
    {
        let connection = open_disk(&path, OpenMode::Create, ConnectionOptions::default()).unwrap();
        assert_eq!(pragma_string(&connection, "locking_mode"), "exclusive");
        assert_eq!(pragma_string(&connection, "journal_mode"), "wal");
        assert_eq!(
            connection
                .pragma_query_value(None, "synchronous", |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .pragma_query_value(None, "cache_size", |r| r.get::<_, i64>(0))
                .unwrap(),
            -2048
        );
        connection.execute_batch("CREATE TABLE saved(value TEXT); BEGIN; INSERT INTO saved VALUES('committed'); COMMIT;").unwrap();
    }
    let connection = open_disk(&path, OpenMode::Existing, ConnectionOptions::default()).unwrap();
    let value: String = connection
        .query_row("SELECT value FROM saved", [], |row| row.get(0))
        .unwrap();
    assert_eq!(value, "committed");
}

#[test]
fn recovery_never_creates_a_missing_database_or_resets_corrupt_data() {
    let directory = TestDirectory::new();
    let path = directory.database();
    assert!(open_disk(&path, OpenMode::Existing, ConnectionOptions::default()).is_err());
    assert!(!path.exists());
    let corrupt = b"this is deliberately not a SQLite database";
    fs::write(&path, corrupt).unwrap();
    let error = open_disk(&path, OpenMode::Existing, ConnectionOptions::default()).unwrap_err();
    assert!(
        matches!(error, ConnectionError::Sqlite(ref error) if classify_error(error) == ErrorClass::Corruption)
    );
    assert_eq!(fs::read(&path).unwrap(), corrupt);
}

#[test]
fn private_memory_connections_and_temporary_tables_have_no_disk_paths() {
    let first = open_private_memory(ConnectionOptions::default()).unwrap();
    let second = open_private_memory(ConnectionOptions::default()).unwrap();
    first.execute_batch("CREATE TABLE saved(value); INSERT INTO saved VALUES(1); CREATE TEMP TABLE scratch(value);").unwrap();
    assert_eq!(pragma_string(&first, "journal_mode"), "memory");
    assert_eq!(
        first
            .pragma_query_value(None, "temp_store", |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    let paths = first
        .prepare("PRAGMA database_list")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(2))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(paths, [String::new(), String::new()]);
    assert!(second.prepare("SELECT * FROM saved").is_err());
}

#[test]
fn schema_version_must_match_without_implicitly_upgrading() {
    let connection = open_private_memory(ConnectionOptions::default()).unwrap();
    assert!(matches!(
        require_schema_version(&connection, 1),
        Err(ConnectionError::SchemaVersion { actual: 0, .. })
    ));
    connection.pragma_update(None, "user_version", 1).unwrap();
    require_schema_version(&connection, 1).unwrap();
    connection.pragma_update(None, "user_version", 2).unwrap();
    assert!(matches!(
        require_schema_version(&connection, 1),
        Err(ConnectionError::SchemaVersion { actual: 2, .. })
    ));
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn constraint_classification_preserves_transaction_state() {
    let connection = open_private_memory(ConnectionOptions::default()).unwrap();
    connection.execute_batch("CREATE TABLE saved(value INTEGER UNIQUE); BEGIN; SAVEPOINT request; INSERT INTO saved VALUES(1);").unwrap();
    let error = connection
        .execute("INSERT INTO saved VALUES(1)", [])
        .unwrap_err();
    assert_eq!(classify_error(&error), ErrorClass::Constraint);
    assert!(!connection.is_autocommit());
    connection
        .execute_batch("ROLLBACK TO request; RELEASE request; INSERT INTO saved VALUES(2); COMMIT;")
        .unwrap();
    assert_eq!(
        connection
            .query_row("SELECT value FROM saved", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn invalid_cache_budget_fails_before_any_schema_is_created() {
    let options = ConnectionOptions {
        cache_kib: 0,
        ..ConnectionOptions::default()
    };
    assert!(matches!(
        open_private_memory(options),
        Err(ConnectionError::InvalidCacheSize)
    ));
    // A path is passed verbatim, never interpreted as a SQLite URI.
    let directory = TestDirectory::new();
    let path = directory.0.join(Path::new("file:literal.sqlite"));
    if cfg!(unix) {
        let connection = open_disk(&path, OpenMode::Create, ConnectionOptions::default()).unwrap();
        drop(connection);
        assert!(path.exists());
    }
}
