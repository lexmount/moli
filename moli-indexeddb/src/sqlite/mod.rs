//! The SQL database owner. Every method runs on its assigned background worker.
//!
//! This owner keeps only committed schema metadata in memory. One Web
//! transaction owns the connection at a time; records and rollback state stay
//! in SQLite. The renderer supplies already extracted index keys.

mod metadata;
mod query;
mod records;
mod schema;
pub mod service;
mod values;

use metadata::{MetadataBudget, MetadataReservation};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use rusqlite::{Connection, OptionalExtension};

use crate::{
    IndexOptions, IndexedDbError, Key, ObjectStoreOptions, TransactionHandle, TransactionMode,
};

pub use query::{Query, QuerySource, Seek};
pub use records::RecordWrite;

pub const SCHEMA_VERSION: u32 = 1;
pub const CHUNK_BYTES: usize = 256 * 1024;
const REQUEST_SAVEPOINT: &str = "SAVEPOINT idb_request";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseIdentity {
    pub owner_key: Vec<u8>,
    pub name: String,
    pub generation: [u8; 16],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexMetadata {
    pub id: i64,
    pub options: IndexOptions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreMetadata {
    pub id: i64,
    pub options: ObjectStoreOptions,
    pub indexes: BTreeMap<String, IndexMetadata>,
}

#[derive(Debug)]
pub struct StoreDefinitions {
    stores: BTreeMap<String, StoreMetadata>,
    _reservation: MetadataReservation,
}

impl std::ops::Deref for StoreDefinitions {
    type Target = BTreeMap<String, StoreMetadata>;
    fn deref(&self) -> &Self::Target {
        &self.stores
    }
}
impl PartialEq for StoreDefinitions {
    fn eq(&self, other: &Self) -> bool {
        self.stores == other.stores
    }
}
impl Eq for StoreDefinitions {}

#[derive(Debug)]
pub struct DatabaseMetadata {
    pub name: String,
    pub version: u64,
    pub usage: u64,
    pub stores: Arc<StoreDefinitions>,
    _reservation: MetadataReservation,
}

impl PartialEq for DatabaseMetadata {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.version == other.version
            && self.usage == other.usage
            && self.stores == other.stores
    }
}
impl Eq for DatabaseMetadata {}

struct ActiveTransaction {
    handle: TransactionHandle,
    mode: TransactionMode,
    stores: BTreeSet<String>,
    pending: Option<PendingWrite>,
}

struct PendingWrite {
    id: u64,
    store_id: i64,
    key: Key,
}

/// A request SAVEPOINT may survive a round trip to the renderer for inline-key
/// injection and final serialization. Its token prevents stale completions
/// from writing into a later request or transaction.
#[derive(Debug, Clone)]
pub struct PreparedWrite {
    pub id: u64,
    pub key: Key,
}

#[derive(Debug, Clone)]
pub struct IndexKeys {
    pub name: String,
    pub keys: Vec<Key>,
}

pub struct Database {
    connection: Connection,
    identity: DatabaseIdentity,
    committed: Arc<DatabaseMetadata>,
    metadata_budget: MetadataBudget,
    active: Option<ActiveTransaction>,
    next_request: u64,
}

impl Database {
    pub fn create(
        mut connection: Connection,
        identity: DatabaseIdentity,
    ) -> Result<Self, IndexedDbError> {
        Self::initialize(&mut connection, &identity)?;
        Self::open(connection, identity)
    }

    fn initialize(
        connection: &mut Connection,
        identity: &DatabaseIdentity,
    ) -> Result<(), IndexedDbError> {
        if identity.owner_key.is_empty() {
            return Err(IndexedDbError::Data(
                "empty IndexedDB owner identity".into(),
            ));
        }
        let existing: i64 =
            connection.query_row("SELECT count(*) FROM sqlite_schema", [], |row| row.get(0))?;
        if existing != 0 {
            return Err(corruption(
                "cannot create IndexedDB over an existing schema",
            ));
        }
        {
            let transaction = connection.transaction()?;
            transaction.execute_batch(include_str!("schema.sql"))?;
            transaction.execute(
                "INSERT INTO database_info VALUES(1, 'indexeddb', ?1, 1)",
                [&identity.owner_key],
            )?;
            transaction.execute(
                "INSERT INTO database_metadata(singleton, name, generation, version, state, key_format, key_path_format, value_format, usage) VALUES(1, ?1, ?2, 0, 'creating', 1, 1, 1, ?3)",
                rusqlite::params![identity.name, identity.generation.as_slice(), sql_integer(identity.name.len() as u64 + 8)?],
            )?;
            transaction.commit()?;
        }
        Ok(())
    }

    pub fn open(
        connection: Connection,
        identity: DatabaseIdentity,
    ) -> Result<Self, IndexedDbError> {
        Self::open_with_budget(connection, identity, MetadataBudget::default())
    }

    fn open_with_budget(
        connection: Connection,
        identity: DatabaseIdentity,
        metadata_budget: MetadataBudget,
    ) -> Result<Self, IndexedDbError> {
        moli_sqlite::require_schema_version(&connection, SCHEMA_VERSION)
            .map_err(|error| corruption(&error.to_string()))?;
        let saved: Option<Vec<u8>> = connection.query_row(
            "SELECT owner_key FROM database_info WHERE singleton=1 AND backend='indexeddb' AND identity_format=1", [], |row| row.get(0),
        ).optional()?;
        if saved.as_deref() != Some(identity.owner_key.as_slice()) {
            return Err(corruption("IndexedDB owner identity mismatch"));
        }
        let generation: Vec<u8> = connection.query_row(
            "SELECT generation FROM database_metadata WHERE singleton=1 AND name=?1",
            [&identity.name],
            |row| row.get(0),
        )?;
        if generation != identity.generation {
            return Err(corruption("IndexedDB generation mismatch"));
        }
        let committed = schema::read_metadata(&connection, &metadata_budget)?;
        Ok(Self {
            connection,
            identity,
            committed,
            metadata_budget,
            active: None,
            next_request: 1,
        })
    }

    pub fn identity(&self) -> &DatabaseIdentity {
        &self.identity
    }
    pub fn committed(&self) -> &Arc<DatabaseMetadata> {
        &self.committed
    }
    pub fn is_transaction_active(&self) -> bool {
        self.active_transaction().is_some()
    }

    fn active_transaction(&self) -> Option<TransactionHandle> {
        (!self.connection.is_autocommit())
            .then(|| self.active.as_ref().map(|tx| tx.handle))
            .flatten()
    }

    /// SQLite may roll back a transaction without going through our write
    /// SAVEPOINT helper (for example while a read fails). Retire that owner as
    /// soon as the connection reports autocommit again. An orphan SQL BEGIN
    /// must be rolled back or its connection retired before releasing admission.
    fn reconcile_transaction(&mut self) -> Result<Option<TransactionHandle>, IndexedDbError> {
        if self.connection.is_autocommit() {
            self.active = None;
            return Ok(None);
        }
        if let Some(transaction) = &self.active {
            return Ok(Some(transaction.handle));
        }
        self.connection.execute_batch("ROLLBACK")?;
        Ok(None)
    }

    pub fn begin(
        &mut self,
        handle: TransactionHandle,
        mode: TransactionMode,
        stores: BTreeSet<String>,
        upgrade_version: Option<u64>,
    ) -> Result<(), IndexedDbError> {
        if self.active.is_some() || !self.connection.is_autocommit() {
            return Err(IndexedDbError::InvalidState(
                "database already has a started transaction".into(),
            ));
        }
        let version = if mode == TransactionMode::VersionChange {
            let version = upgrade_version
                .ok_or_else(|| IndexedDbError::Version("upgrade version is required".into()))?;
            if version <= self.committed.version {
                return Err(IndexedDbError::Version(
                    "upgrade version must increase".into(),
                ));
            }
            Some(sql_integer(version)?)
        } else {
            if upgrade_version.is_some() || self.committed.version == 0 || stores.is_empty() {
                return Err(IndexedDbError::InvalidState(
                    "regular transaction requires a live database and nonempty scope".into(),
                ));
            }
            for name in &stores {
                if !self.committed.stores.contains_key(name) {
                    return Err(IndexedDbError::NotFound(format!(
                        "object store `{name}` was not found"
                    )));
                }
            }
            None
        };
        self.connection
            .execute_batch(if mode == TransactionMode::ReadOnly {
                "BEGIN DEFERRED"
            } else {
                "BEGIN IMMEDIATE"
            })?;
        if let Some(version) = version
            && let Err(error) = self.connection.execute(
                "UPDATE database_metadata SET version=?1, state='live' WHERE singleton=1",
                [version],
            )
        {
            let _ = self.connection.execute_batch("ROLLBACK");
            return Err(error.into());
        }
        self.active = Some(ActiveTransaction {
            handle,
            mode,
            stores,
            pending: None,
        });
        Ok(())
    }

    pub fn candidate_metadata(
        &self,
        transaction: TransactionHandle,
    ) -> Result<Arc<DatabaseMetadata>, IndexedDbError> {
        if self.transaction(transaction)?.mode == TransactionMode::VersionChange {
            return schema::read_metadata(&self.connection, &self.metadata_budget);
        }
        let usage: Option<i64> = self.connection.query_row(
            "SELECT usage FROM database_metadata WHERE singleton=1 AND name=?1 AND version=?2 AND state='live' AND key_format=1 AND key_path_format=1 AND value_format=1",
            rusqlite::params![self.committed.name, sql_integer(self.committed.version)?],
            |row| row.get(0),
        ).optional()?;
        let usage = usage
            .filter(|usage| *usage >= 0)
            .ok_or_else(|| corruption("invalid IndexedDB committed metadata"))?
            as u64;
        if usage == self.committed.usage {
            return Ok(self.committed.clone());
        }
        self.committed.with_usage(usage, &self.metadata_budget)
    }

    pub fn commit(
        &mut self,
        transaction: TransactionHandle,
    ) -> Result<Arc<DatabaseMetadata>, IndexedDbError> {
        self.ready(transaction)?;
        // Everything needed for publication is prepared before irreversible COMMIT.
        let candidate = match self.candidate_metadata(transaction) {
            Ok(metadata) => metadata,
            Err(error) => {
                self.rollback_after_failure();
                return Err(error);
            }
        };
        if let Err(error) = self.connection.execute_batch("COMMIT") {
            let failure = error.into();
            self.rollback_after_failure();
            return Err(failure);
        }
        self.active = None;
        self.committed = candidate;
        Ok(self.committed.clone())
    }

    pub fn abort(&mut self, transaction: TransactionHandle) -> Result<(), IndexedDbError> {
        if self
            .active
            .as_ref()
            .is_none_or(|active| active.handle != transaction)
        {
            return Err(IndexedDbError::TransactionInactive(
                "IndexedDB SQL transaction is not active".into(),
            ));
        }
        let result = if self.connection.is_autocommit() {
            Ok(())
        } else {
            self.connection.execute_batch("ROLLBACK")
        };
        self.active = None;
        result.map_err(Into::into)
    }

    /// Shutdown runs on the database worker, before the profile lock is released.
    pub fn close(mut self) -> Result<(), IndexedDbError> {
        if !self.connection.is_autocommit() {
            self.connection.execute_batch("ROLLBACK")?;
        }
        self.active = None;
        self.connection.close().map_err(|(_, error)| error.into())
    }

    fn transaction(&self, handle: TransactionHandle) -> Result<&ActiveTransaction, IndexedDbError> {
        self.active
            .as_ref()
            .filter(|tx| tx.handle == handle && !self.connection.is_autocommit())
            .ok_or_else(|| {
                IndexedDbError::TransactionInactive(
                    "IndexedDB SQL transaction is not active".into(),
                )
            })
    }

    fn ready(&self, handle: TransactionHandle) -> Result<&ActiveTransaction, IndexedDbError> {
        let transaction = self.transaction(handle)?;
        if transaction.pending.is_some() {
            return Err(IndexedDbError::InvalidState(
                "a write request is awaiting its owner".into(),
            ));
        }
        Ok(transaction)
    }

    fn writable(&self, handle: TransactionHandle) -> Result<(), IndexedDbError> {
        if self.ready(handle)?.mode == TransactionMode::ReadOnly {
            return Err(IndexedDbError::ReadOnly("transaction is readonly".into()));
        }
        Ok(())
    }

    fn store_id(&self, handle: TransactionHandle, name: &str) -> Result<i64, IndexedDbError> {
        let transaction = self.transaction(handle)?;
        if transaction.mode != TransactionMode::VersionChange && !transaction.stores.contains(name)
        {
            return Err(IndexedDbError::InvalidState(format!(
                "transaction does not cover object store `{name}`"
            )));
        }
        self.connection
            .query_row(
                "SELECT store_id FROM object_stores WHERE name=?1",
                [name],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| IndexedDbError::NotFound(format!("object store `{name}` was not found")))
    }

    fn request<T>(
        &mut self,
        transaction: TransactionHandle,
        operation: impl FnOnce(&mut Self) -> Result<T, IndexedDbError>,
    ) -> Result<T, IndexedDbError> {
        self.writable(transaction)?;
        self.open_request()?;
        let outcome = operation(self);
        self.finish_request(outcome)
    }

    fn open_request(&mut self) -> Result<(), IndexedDbError> {
        if let Err(error) = self.connection.execute_batch(REQUEST_SAVEPOINT) {
            if self.connection.is_autocommit() {
                self.active = None;
                return Err(IndexedDbError::TransactionInactive(format!(
                    "SQLite aborted the transaction: {error}"
                )));
            }
            return Err(error.into());
        }
        Ok(())
    }

    fn finish_request<T>(
        &mut self,
        outcome: Result<T, IndexedDbError>,
    ) -> Result<T, IndexedDbError> {
        match outcome {
            Ok(value) => match self.connection.execute_batch("RELEASE idb_request") {
                Ok(()) => Ok(value),
                Err(error) => {
                    self.rollback_after_failure();
                    Err(IndexedDbError::TransactionInactive(format!(
                        "request release failed: {error}"
                    )))
                }
            },
            Err(error) => {
                self.rollback_request()?;
                Err(error)
            }
        }
    }

    fn rollback_request(&mut self) -> Result<(), IndexedDbError> {
        if self.connection.is_autocommit() {
            self.active = None;
            return Err(IndexedDbError::TransactionInactive(
                "SQLite aborted the transaction".into(),
            ));
        }
        if let Err(error) = self
            .connection
            .execute_batch("ROLLBACK TO idb_request; RELEASE idb_request")
        {
            self.rollback_after_failure();
            return Err(IndexedDbError::TransactionInactive(format!(
                "request rollback failed: {error}"
            )));
        }
        Ok(())
    }

    fn rollback_after_failure(&mut self) {
        if !self.connection.is_autocommit() {
            let _ = self.connection.execute_batch("ROLLBACK");
        }
        self.active = None;
    }

    fn adjust_usage(&self, delta: i128) -> Result<(), IndexedDbError> {
        let current: i64 = self.connection.query_row(
            "SELECT usage FROM database_metadata WHERE singleton=1",
            [],
            |row| row.get(0),
        )?;
        let next = i64::try_from(i128::from(current) + delta)
            .ok()
            .filter(|value| *value >= 0)
            .ok_or_else(|| corruption("IndexedDB logical usage overflow"))?;
        self.connection.execute(
            "UPDATE database_metadata SET usage=?1 WHERE singleton=1",
            [next],
        )?;
        Ok(())
    }

    fn next_id(&self, column: &'static str) -> Result<i64, IndexedDbError> {
        let id: i64 = self.connection.query_row(
            &format!("SELECT {column} FROM database_metadata WHERE singleton=1"),
            [],
            |row| row.get(0),
        )?;
        let next = id
            .checked_add(1)
            .filter(|_| id > 0)
            .ok_or_else(|| corruption("IndexedDB ID counter exhausted"))?;
        self.connection.execute(
            &format!("UPDATE database_metadata SET {column}=?1 WHERE singleton=1"),
            [next],
        )?;
        Ok(id)
    }
}

fn sql_integer(value: u64) -> Result<i64, IndexedDbError> {
    i64::try_from(value).map_err(|_| {
        IndexedDbError::Data("IndexedDB integer exceeds SQLite's supported range".into())
    })
}

fn corruption(message: &str) -> IndexedDbError {
    IndexedDbError::Corruption(message.into())
}

impl From<rusqlite::Error> for IndexedDbError {
    fn from(error: rusqlite::Error) -> Self {
        match moli_sqlite::classify_error(&error) {
            moli_sqlite::ErrorClass::Constraint => Self::Constraint(error.to_string()),
            moli_sqlite::ErrorClass::Corruption => Self::Corruption(error.to_string()),
            _ => Self::Io(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests;
