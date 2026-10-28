use std::collections::BTreeSet;

use moli_sqlite::TaskKind;

use super::super::*;
use crate::{IndexedDbValue, TransactionRequestHandle};

/// Only native, owned payloads cross the database worker boundary.
pub enum Command {
    Begin {
        transaction: TransactionHandle,
        mode: TransactionMode,
        stores: BTreeSet<String>,
        version: Option<u64>,
        admission: Option<TransactionRequestHandle>,
    },
    CreateStore {
        transaction: TransactionHandle,
        name: String,
        options: ObjectStoreOptions,
    },
    DeleteStore {
        transaction: TransactionHandle,
        name: String,
    },
    CreateIndex {
        transaction: TransactionHandle,
        store: String,
        name: String,
        options: IndexOptions,
    },
    DeleteIndex {
        transaction: TransactionHandle,
        store: String,
        name: String,
    },
    /// Only an auto-generated numeric inline key needs a renderer round trip.
    PrepareWrite {
        transaction: TransactionHandle,
        store: String,
        add_only: bool,
    },
    Write {
        transaction: TransactionHandle,
        store: String,
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
        store: String,
        name: String,
        primary_key: Key,
        keys: Vec<Key>,
    },
    Query {
        transaction: TransactionHandle,
        store: String,
        query: Query,
    },
    Count {
        transaction: TransactionHandle,
        store: String,
        source: QuerySource,
        range: crate::KeyRange,
    },
    Delete {
        transaction: TransactionHandle,
        store: String,
        range: crate::KeyRange,
    },
    Commit {
        transaction: TransactionHandle,
    },
    Abort {
        transaction: TransactionHandle,
    },
    /// Retire this logical SQL database, rather than one Web IDBDatabase wrapper.
    Close,
}

impl Command {
    pub fn kind(&self) -> TaskKind {
        match self {
            Self::Commit { .. } | Self::Abort { .. } | Self::CancelWrite { .. } | Self::Close => {
                TaskKind::Control
            }
            _ => TaskKind::Data,
        }
    }

    /// Resident native input, including Vec capacities and nested key buffers.
    /// Schema/identity data shared through Arc are accounted by the owner cache.
    pub fn input_bytes(&self) -> usize {
        let base = std::mem::size_of::<Self>();
        let extra = match self {
            Self::Begin { stores, .. } => stores.iter().fold(0_usize, |size, name| {
                size.saturating_add(name.capacity()).saturating_add(512)
            }),
            Self::CreateStore { name, options, .. } => name
                .capacity()
                .saturating_add(options.key_path.as_ref().map_or(0, path_bytes)),
            Self::DeleteStore { name, .. } => name.capacity(),
            Self::CreateIndex {
                store,
                name,
                options,
                ..
            } => store
                .capacity()
                .saturating_add(name.capacity())
                .saturating_add(path_bytes(&options.key_path)),
            Self::DeleteIndex { store, name, .. } => {
                store.capacity().saturating_add(name.capacity())
            }
            Self::PrepareWrite { store, .. } => store.capacity(),
            Self::Write {
                store,
                key,
                value,
                indexes,
                ..
            } => store
                .capacity()
                .saturating_add(key.as_ref().map_or(0, key_bytes))
                .saturating_add(value_bytes(value))
                .saturating_add(indexes_bytes(indexes)),
            Self::FinishWrite { value, indexes, .. } => {
                value_bytes(value).saturating_add(indexes_bytes(indexes))
            }
            Self::BackfillIndex {
                store,
                name,
                primary_key,
                keys,
                ..
            } => store
                .capacity()
                .saturating_add(name.capacity())
                .saturating_add(key_bytes(primary_key))
                .saturating_add(keys_bytes(keys)),
            Self::Query { store, query, .. } => store
                .capacity()
                .saturating_add(source_bytes(&query.source))
                .saturating_add(range_bytes(&query.range))
                .saturating_add(query.seek.as_ref().map_or(0, |seek| {
                    key_bytes(&seek.key)
                        .saturating_add(seek.primary_key.as_ref().map_or(0, key_bytes))
                })),
            Self::Count {
                store,
                source,
                range,
                ..
            } => store
                .capacity()
                .saturating_add(source_bytes(source))
                .saturating_add(range_bytes(range)),
            Self::Delete { store, range, .. } => {
                store.capacity().saturating_add(range_bytes(range))
            }
            Self::Commit { .. } | Self::Abort { .. } | Self::CancelWrite { .. } | Self::Close => 0,
        };
        base.saturating_add(extra)
    }
}

fn source_bytes(source: &QuerySource) -> usize {
    match source {
        QuerySource::ObjectStore => 0,
        QuerySource::Index(name) => name.capacity(),
    }
}
fn indexes_bytes(indexes: &Vec<IndexKeys>) -> usize {
    indexes
        .capacity()
        .saturating_mul(std::mem::size_of::<IndexKeys>())
        .saturating_add(indexes.iter().fold(0_usize, |size, index| {
            size.saturating_add(index.name.capacity())
                .saturating_add(keys_bytes(&index.keys))
        }))
}
fn range_bytes(range: &crate::KeyRange) -> usize {
    range
        .lower
        .as_ref()
        .map_or(0, key_bytes)
        .saturating_add(range.upper.as_ref().map_or(0, key_bytes))
}
fn path_bytes(path: &crate::KeyPath) -> usize {
    match path {
        crate::KeyPath::String(value) => value.capacity(),
        crate::KeyPath::Sequence(values) => values
            .capacity()
            .saturating_mul(std::mem::size_of::<String>())
            .saturating_add(
                values
                    .iter()
                    .fold(0_usize, |size, value| size.saturating_add(value.capacity())),
            ),
    }
}

fn key_bytes(key: &Key) -> usize {
    key.heap_bytes()
}
fn keys_bytes(keys: &Vec<Key>) -> usize {
    keys.capacity()
        .saturating_mul(std::mem::size_of::<Key>())
        .saturating_add(
            keys.iter()
                .fold(0usize, |n, key| n.saturating_add(key.heap_bytes())),
        )
}

fn value_bytes(value: &IndexedDbValue) -> usize {
    value.heap_bytes()
}
