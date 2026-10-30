use std::collections::BTreeSet;

use super::super::*;
use crate::{IndexedDbValue, TransactionRequestHandle};

/// Only native, owned payloads cross the database worker boundary.
pub enum Command {
    Begin {
        transaction: TransactionHandle,
        mode: TransactionMode,
        stores: BTreeSet<crate::IndexedDbName>,
        version: Option<u64>,
        admission: Option<TransactionRequestHandle>,
    },
    CreateStore {
        transaction: TransactionHandle,
        name: crate::IndexedDbName,
        options: ObjectStoreOptions,
    },
    DeleteStore {
        transaction: TransactionHandle,
        name: crate::IndexedDbName,
    },
    RenameStore {
        transaction: TransactionHandle,
        name: crate::IndexedDbName,
        new_name: crate::IndexedDbName,
    },
    CreateIndex {
        transaction: TransactionHandle,
        store: crate::IndexedDbName,
        name: crate::IndexedDbName,
        options: IndexOptions,
    },
    DeleteIndex {
        transaction: TransactionHandle,
        store: crate::IndexedDbName,
        name: crate::IndexedDbName,
    },
    RenameIndex {
        transaction: TransactionHandle,
        store: crate::IndexedDbName,
        name: crate::IndexedDbName,
        new_name: crate::IndexedDbName,
    },
    /// Only an auto-generated numeric inline key needs a renderer round trip.
    PrepareWrite {
        transaction: TransactionHandle,
        store: crate::IndexedDbName,
        add_only: bool,
    },
    Write {
        transaction: TransactionHandle,
        store: crate::IndexedDbName,
        key: Option<Key>,
        value: IndexedDbValue,
        indexes: Vec<IndexKeys>,
        add_only: bool,
        quota: Option<u64>,
    },
    FinishWrite {
        transaction: TransactionHandle,
        request: u64,
        value: IndexedDbValue,
        indexes: Vec<IndexKeys>,
        quota: Option<u64>,
    },
    CancelWrite {
        transaction: TransactionHandle,
        request: u64,
    },
    BackfillIndex {
        transaction: TransactionHandle,
        store: crate::IndexedDbName,
        name: crate::IndexedDbName,
        primary_key: Key,
        keys: Vec<Key>,
    },
    Query {
        transaction: TransactionHandle,
        store: crate::IndexedDbName,
        query: Query,
    },
    Count {
        transaction: TransactionHandle,
        store: crate::IndexedDbName,
        source: QuerySource,
        range: crate::KeyRange,
    },
    Delete {
        transaction: TransactionHandle,
        store: crate::IndexedDbName,
        range: crate::KeyRange,
    },
    Commit {
        transaction: TransactionHandle,
        quota: Option<QuotaCheck>,
    },
    Abort {
        transaction: TransactionHandle,
    },
    /// Retire this logical SQL database, rather than one Web IDBDatabase wrapper.
    Close,
    /// Close and durably retire the whole catalog generation.
    DeleteDatabase,
}

impl Command {
    pub(super) fn transaction(&self) -> Option<TransactionHandle> {
        match self {
            Self::Begin { transaction, .. }
            | Self::CreateStore { transaction, .. }
            | Self::DeleteStore { transaction, .. }
            | Self::RenameStore { transaction, .. }
            | Self::CreateIndex { transaction, .. }
            | Self::DeleteIndex { transaction, .. }
            | Self::RenameIndex { transaction, .. }
            | Self::PrepareWrite { transaction, .. }
            | Self::Write { transaction, .. }
            | Self::FinishWrite { transaction, .. }
            | Self::CancelWrite { transaction, .. }
            | Self::BackfillIndex { transaction, .. }
            | Self::Query { transaction, .. }
            | Self::Count { transaction, .. }
            | Self::Delete { transaction, .. }
            | Self::Commit { transaction, .. }
            | Self::Abort { transaction } => Some(*transaction),
            Self::Close | Self::DeleteDatabase => None,
        }
    }
}
