use std::{fmt, path::Path, time::Duration};

use rusqlite::{Connection, ErrorCode, OpenFlags};

#[derive(Debug, Clone, Copy)]
pub struct ConnectionOptions {
    pub cache_kib: u32,
    pub statement_cache_capacity: usize,
}

impl Default for ConnectionOptions {
    fn default() -> Self {
        Self {
            cache_kib: 2048,
            statement_cache_capacity: 32,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum OpenMode {
    /// The caller has exclusively allocated the containing generation directory.
    Create,
    /// Never create a missing database while recovering an existing identity.
    Existing,
}

#[derive(Debug)]
pub enum ConnectionError {
    Sqlite(rusqlite::Error),
    InvalidCacheSize,
    Configuration {
        setting: &'static str,
        expected: &'static str,
        actual: String,
    },
    SchemaVersion {
        expected: u32,
        actual: i64,
    },
}

impl fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => error.fmt(f),
            Self::InvalidCacheSize => {
                f.write_str("SQLite cache size must be between 1 and i32::MAX KiB")
            }
            Self::Configuration {
                setting,
                expected,
                actual,
            } => write!(f, "SQLite {setting}: expected {expected}, got {actual}"),
            Self::SchemaVersion { expected, actual } => {
                write!(
                    f,
                    "SQLite schema version: expected {expected}, got {actual}"
                )
            }
        }
    }
}

impl std::error::Error for ConnectionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for ConnectionError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

/// Open only on the database's assigned background worker.
pub fn open_disk(
    path: &Path,
    mode: OpenMode,
    options: ConnectionOptions,
) -> Result<Connection, ConnectionError> {
    let mut flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    if matches!(mode, OpenMode::Create) {
        flags |= OpenFlags::SQLITE_OPEN_CREATE;
    }
    let connection = Connection::open_with_flags(path, flags)?;
    configure_common(&connection, options)?;
    connection.pragma_update(None, "locking_mode", "EXCLUSIVE")?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    require_setting(&connection, "locking_mode", "exclusive")?;
    require_setting(&connection, "journal_mode", "wal")?;
    let synchronous: i64 = connection.pragma_query_value(None, "synchronous", |row| row.get(0))?;
    if synchronous != 1 {
        return Err(ConnectionError::Configuration {
            setting: "synchronous",
            expected: "NORMAL (1)",
            actual: synchronous.to_string(),
        });
    }
    Ok(connection)
}

/// A private database, retained by its owner until deletion or shutdown.
pub fn open_private_memory(options: ConnectionOptions) -> Result<Connection, ConnectionError> {
    let connection = Connection::open_in_memory()?;
    configure_common(&connection, options)?;
    require_setting(&connection, "journal_mode", "memory")?;
    Ok(connection)
}

fn configure_common(
    connection: &Connection,
    options: ConnectionOptions,
) -> Result<(), ConnectionError> {
    if options.cache_kib == 0 || options.cache_kib > i32::MAX as u32 {
        return Err(ConnectionError::InvalidCacheSize);
    }
    // One connection per owner and a profile process lock: do not implicitly
    // block a worker waiting for another connection to release this file.
    connection.busy_timeout(Duration::ZERO)?;
    connection.pragma_update(None, "temp_store", "MEMORY")?;
    connection.pragma_update(None, "cache_size", -i64::from(options.cache_kib))?;
    connection.set_prepared_statement_cache_capacity(options.statement_cache_capacity);
    let temp_store: i64 = connection.pragma_query_value(None, "temp_store", |row| row.get(0))?;
    if temp_store != 2 {
        return Err(ConnectionError::Configuration {
            setting: "temp_store",
            expected: "MEMORY (2)",
            actual: temp_store.to_string(),
        });
    }
    let cache_size: i64 = connection.pragma_query_value(None, "cache_size", |row| row.get(0))?;
    if cache_size != -i64::from(options.cache_kib) {
        return Err(ConnectionError::Configuration {
            setting: "cache_size",
            expected: "configured negative KiB budget",
            actual: cache_size.to_string(),
        });
    }
    Ok(())
}

fn require_setting(
    connection: &Connection,
    setting: &'static str,
    expected: &'static str,
) -> Result<(), ConnectionError> {
    let actual: String = connection.pragma_query_value(None, setting, |row| row.get(0))?;
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(ConnectionError::Configuration {
            setting,
            expected,
            actual,
        })
    }
}

pub fn require_schema_version(
    connection: &Connection,
    expected: u32,
) -> Result<(), ConnectionError> {
    let actual: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if actual == i64::from(expected) {
        Ok(())
    } else {
        Err(ConnectionError::SchemaVersion { expected, actual })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    Constraint,
    Busy,
    Resource,
    Corruption,
    Io,
    Interrupted,
    Other,
}

/// Classification does not decide whether a transaction survived. After an
/// error, the owner must inspect `Connection::is_autocommit()` before attempting
/// request SAVEPOINT rollback or accepting another request in that transaction.
pub fn classify_error(error: &rusqlite::Error) -> ErrorClass {
    match error.sqlite_error_code() {
        Some(ErrorCode::ConstraintViolation) => ErrorClass::Constraint,
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => ErrorClass::Busy,
        Some(ErrorCode::DiskFull | ErrorCode::OutOfMemory | ErrorCode::TooBig) => {
            ErrorClass::Resource
        }
        Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) => ErrorClass::Corruption,
        Some(ErrorCode::SystemIoFailure | ErrorCode::CannotOpen | ErrorCode::ReadOnly) => {
            ErrorClass::Io
        }
        Some(ErrorCode::OperationInterrupted) => ErrorClass::Interrupted,
        _ => ErrorClass::Other,
    }
}

#[cfg(test)]
mod tests;
