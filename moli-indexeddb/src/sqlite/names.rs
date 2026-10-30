//! Database names use UTF-16BE BLOBs in schema v2. Store/index names retain
//! their schema-v1 representation until their independent name migrations.
use super::*;
use crate::IndexedDbName;

pub(super) fn schema_version(connection: &Connection) -> Result<u32, IndexedDbError> {
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    match version {
        1 | 2 => Ok(version as u32),
        _ => Err(corruption("unsupported IndexedDB SQL schema version")),
    }
}

pub(super) fn encode(name: &IndexedDbName) -> Result<Vec<u8>, IndexedDbError> {
    let len = name
        .as_utf16()
        .len()
        .checked_mul(2)
        .ok_or_else(|| IndexedDbError::Resource("IndexedDB name size overflow".into()))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(len)
        .map_err(|_| IndexedDbError::Resource("Unable to allocate IndexedDB name".into()))?;
    for unit in name.as_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    Ok(bytes)
}

pub(super) fn decode(bytes: &[u8]) -> Result<IndexedDbName, IndexedDbError> {
    if !bytes.len().is_multiple_of(2) {
        return Err(corruption("invalid UTF-16 database name"));
    }
    let mut units = Vec::new();
    units
        .try_reserve_exact(bytes.len() / 2)
        .map_err(|_| IndexedDbError::Resource("Unable to allocate IndexedDB name".into()))?;
    for pair in bytes.chunks_exact(2) {
        units.push(u16::from_be_bytes([pair[0], pair[1]]));
    }
    Ok(IndexedDbName::from_utf16(units))
}

pub(super) fn read_name(
    row: &rusqlite::Row<'_>,
    column: usize,
    version: u32,
) -> Result<IndexedDbName, IndexedDbError> {
    match (version, row.get_ref(column)?) {
        (1, rusqlite::types::ValueRef::Text(bytes)) => {
            let text = std::str::from_utf8(bytes)
                .map_err(|_| corruption("invalid legacy database name"))?;
            let mut units = Vec::new();
            units.try_reserve_exact(text.len()).map_err(|_| {
                IndexedDbError::Resource("Unable to allocate IndexedDB name".into())
            })?;
            units.extend(text.encode_utf16());
            Ok(IndexedDbName::from_utf16(units))
        }
        (2, rusqlite::types::ValueRef::Blob(bytes)) => decode(bytes),
        _ => Err(corruption("invalid IndexedDB database name column")),
    }
}

pub(super) fn copy(name: &IndexedDbName) -> Result<IndexedDbName, IndexedDbError> {
    let mut units = Vec::new();
    units
        .try_reserve_exact(name.as_utf16().len())
        .map_err(|_| IndexedDbError::Resource("Unable to allocate IndexedDB name".into()))?;
    units.extend_from_slice(name.as_utf16());
    Ok(IndexedDbName::from_utf16(units))
}

/// Called only after the legacy owner/name/generation have been validated.
/// Table replacement, name conversion and format publication share one SQL
/// transaction. Records and all stable ids are left in their existing tables.
pub(super) fn migrate_legacy(
    connection: &mut Connection,
    name: &IndexedDbName,
) -> Result<(), IndexedDbError> {
    let name = encode(name)?;
    let transaction =
        connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    transaction.execute_batch("ALTER TABLE database_metadata RENAME TO database_metadata_v1")?;
    transaction.execute_batch(include_str!("migrations/v2_metadata.sql"))?;
    transaction.execute(
        "INSERT INTO database_metadata SELECT singleton, ?1, generation, version, state, key_format, key_path_format, value_format, usage, next_store_id, next_index_id, next_record_id, next_object_id FROM database_metadata_v1",
        [name],
    )?;
    transaction.execute_batch("DROP TABLE database_metadata_v1; PRAGMA user_version=2")?;
    transaction.commit()?;
    Ok(())
}
