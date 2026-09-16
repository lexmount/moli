use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::AtomicU64,
};

use crate::{
    DatabaseHandle, IndexedDbValue, Key, KeyPath, TransactionHandle, TransactionMode,
    persistence::IndexedDbPersistenceBackend,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OriginState {
    pub(crate) databases: BTreeMap<String, DatabaseData>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DatabaseData {
    pub(crate) version: u64,
    pub(crate) stores: BTreeMap<String, ObjectStoreData>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ObjectStoreData {
    pub(crate) key_path: Option<KeyPath>,
    pub(crate) auto_increment: bool,
    pub(crate) auto_increment_counter: u64,
    pub(crate) indexes: BTreeMap<String, IndexData>,
    pub(crate) records: BTreeMap<Key, IndexedDbValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct IndexData {
    pub(crate) key_path: KeyPath,
    pub(crate) unique: bool,
    pub(crate) multi_entry: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct DatabaseHandleState {
    pub(crate) origin: String,
    pub(crate) name: String,
    pub(crate) closed: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct TransactionState {
    pub(crate) origin: String,
    pub(crate) db_name: String,
    pub(crate) mode: TransactionMode,
    pub(crate) stores: BTreeSet<String>,
    pub(crate) state: TransactionLifecycle,
    pub(crate) working_copy: DatabaseData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransactionLifecycle {
    Active,
    Committed,
    Aborted,
}

pub struct IndexedDbManager {
    pub(crate) connection_requests: std::sync::Arc<crate::ConnectionRequestQueues>,
    pub(crate) backend: IndexedDbPersistenceBackend,
    pub(crate) origins: BTreeMap<String, OriginState>,
    pub(crate) databases: BTreeMap<DatabaseHandle, DatabaseHandleState>,
    pub(crate) transactions: BTreeMap<TransactionHandle, TransactionState>,
    pub(crate) next_database_handle: AtomicU64,
    pub(crate) next_transaction_handle: AtomicU64,
}
